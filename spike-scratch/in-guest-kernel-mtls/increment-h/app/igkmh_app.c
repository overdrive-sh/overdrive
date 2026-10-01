// SPDX-License-Identifier: BSD-3-Clause
/*
 * Spike C (GH #303, increment-h) guest application. THROWAWAY.
 * An UNMODIFIED-style Linux app: plain blocking TCP sockets, no TLS awareness.
 * The in-guest kernel module transparently mTLS-wraps mesh-bound connects via
 * kTLS; this program never sees keys or records (except the `drain` mode, which
 * exists only to characterise kTLS control-record delivery with recvmsg+cmsg).
 *
 *   igkmg_app <mode> <ip> <port> [tag]
 *     echo        connect; write one REQ line; one plain read(); print
 *     twostep     connect; REQ1/read; pause (for host relay-kill); REQ2/read
 *     serverfirst connect; plain read() the greeting; write REQ; read
 *     deny        connect; print the connect() result (expect failure)
 *     passthrough connect; REQ/read (plain, non-mesh)
 *     drain       connect; REQ; read via recvmsg()+TLS cmsg, skipping non-data
 *                 (NewSessionTicket/KeyUpdate) records; print the app data
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <sys/time.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <arpa/inet.h>
#include <linux/tls.h>

#ifndef SOL_TLS
#define SOL_TLS 282
#endif
#ifndef TLS_GET_RECORD_TYPE
#define TLS_GET_RECORD_TYPE 2
#endif

static int connect_to(const char *ip, int port)
{
	int fd = socket(AF_INET, SOCK_STREAM, 0);
	struct sockaddr_in sin;

	struct timeval tv = { .tv_sec = 3, .tv_usec = 0 };

	if (fd < 0) { printf("IGKMG-APP: socket errno=%d\n", errno); return -1; }
	setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv)); /* bound reads so a kTLS control-record stall can't hang the run */
	memset(&sin, 0, sizeof(sin));
	sin.sin_family = AF_INET;
	sin.sin_port = htons(port);
	inet_pton(AF_INET, ip, &sin.sin_addr);
	if (connect(fd, (struct sockaddr *)&sin, sizeof(sin)) < 0) {
		printf("IGKMG-APP: connect %s:%d FAILED errno=%d (%s)\n", ip, port, errno, strerror(errno));
		close(fd);
		return -1;
	}
	printf("IGKMG-APP: connect %s:%d returned 0 (transparent mTLS installed before return if mesh)\n", ip, port);
	return fd;
}

static void wr(int fd, const char *s)
{
	ssize_t n = write(fd, s, strlen(s));

	printf("IGKMG-APP: write(%zu) rc=%zd: %.*s", strlen(s), n, (int)strlen(s), s);
}

/* plain read(): the unmodified-app behaviour. */
static void rd_plain(int fd, const char *what)
{
	char buf[2048];
	ssize_t n = read(fd, buf, sizeof(buf) - 1);

	if (n < 0) { printf("IGKMG-APP: %s read FAILED errno=%d (%s)\n", what, errno, strerror(errno)); return; }
	if (n == 0) { printf("IGKMG-APP: %s read EOF\n", what); return; }
	buf[n] = 0;
	printf("IGKMG-APP: %s read(%zd) PLAINTEXT: %s%s", what, n, buf, (buf[n-1] == '\n') ? "" : "\n");
}

/* recvmsg()+TLS cmsg: skip control records (NewSessionTicket=22, etc.), return app data. */
static void rd_drain(int fd, const char *what)
{
	for (;;) {
		char buf[2048];
		char cbuf[CMSG_SPACE(sizeof(unsigned char))];
		struct iovec iov = { .iov_base = buf, .iov_len = sizeof(buf) - 1 };
		struct msghdr msg = { .msg_iov = &iov, .msg_iovlen = 1,
				      .msg_control = cbuf, .msg_controllen = sizeof(cbuf) };
		struct cmsghdr *c;
		unsigned char rectype = 23; /* assume app data unless cmsg says otherwise */
		ssize_t n = recvmsg(fd, &msg, 0);

		if (n < 0) { printf("IGKMG-APP: %s recvmsg FAILED errno=%d (%s)\n", what, errno, strerror(errno)); return; }
		if (n == 0) { printf("IGKMG-APP: %s recvmsg EOF\n", what); return; }
		for (c = CMSG_FIRSTHDR(&msg); c; c = CMSG_NXTHDR(&msg, c))
			if (c->cmsg_level == SOL_TLS && c->cmsg_type == TLS_GET_RECORD_TYPE)
				rectype = *(unsigned char *)CMSG_DATA(c);
		if (rectype != 23) {
			printf("IGKMG-APP: %s recvmsg skipped a CONTROL record type=%u (%zd bytes) [NewSessionTicket=22]\n",
			       what, rectype, n);
			continue;
		}
		buf[n] = 0;
		printf("IGKMG-APP: %s recvmsg(%zd) APP-DATA (drained tickets): %s%s",
		       what, n, buf, (buf[n-1] == '\n') ? "" : "\n");
		return;
	}
}

int main(int argc, char **argv)
{
	const char *mode = argc > 1 ? argv[1] : "echo";
	const char *ip = argc > 2 ? argv[2] : "192.168.205.1";
	int port = argc > 3 ? atoi(argv[3]) : 6450;
	const char *tag = argc > 4 ? argv[4] : "1";
	char req[128];
	int fd;

	setvbuf(stdout, NULL, _IONBF, 0);
	printf("IGKMG-APP: START mode=%s dst=%s:%d tag=%s\n", mode, ip, port, tag);

	if (!strcmp(mode, "deny")) {
		ssize_t wn;

		fd = connect_to(ip, port);
		if (fd < 0) {
			printf("IGKMG-APP: DENY-CASE OK: connect refused\n");
			return 0;
		}
		/* The hold is at first I/O (connect runs under the socket lock), so a
		 * policy deny surfaces as the FIRST write/read failing, not connect. */
		snprintf(req, sizeof(req), "IGKM-H-REQ-DENY-%s guest->peer MUST-NOT-ARRIVE\n", tag);
		wn = write(fd, req, strlen(req));
		if (wn < 0)
			printf("IGKMG-APP: DENY-CASE OK: first write refused errno=%d (%s); no application bytes on the wire\n",
			       errno, strerror(errno));
		else
			printf("IGKMG-APP: DENY-CASE UNEXPECTED: first write sent %zd bytes\n", wn);
		close(fd);
		return 0;
	}

	fd = connect_to(ip, port);
	if (fd < 0) return 1;

	if (!strcmp(mode, "serverfirst")) {
		rd_plain(fd, "greeting");
		snprintf(req, sizeof(req), "IGKM-H-REQ-SF-%s guest->peer after greeting\n", tag);
		wr(fd, req);
		rd_plain(fd, "sf-resp");
	} else if (!strcmp(mode, "twostep")) {
		snprintf(req, sizeof(req), "IGKM-H-REQ-A-%s guest->peer exchange-1\n", tag);
		wr(fd, req);
		rd_plain(fd, "exch1");
		printf("IGKMG-APP: PAUSE-FOR-RELAY-KILL (sleeping 5s; host may SIGKILL the relay now)\n");
		sleep(5);
		snprintf(req, sizeof(req), "IGKM-H-REQ-B-%s guest->peer exchange-2-after-relay-gone\n", tag);
		wr(fd, req);
		rd_plain(fd, "exch2");
	} else if (!strcmp(mode, "drain")) {
		snprintf(req, sizeof(req), "IGKM-H-REQ-DR-%s guest->peer drain\n", tag);
		wr(fd, req);
		rd_drain(fd, "drain");
	} else if (!strcmp(mode, "passthrough")) {
		snprintf(req, sizeof(req), "IGKM-H-REQ-PT-%s guest->peer plaintext\n", tag);
		wr(fd, req);
		rd_plain(fd, "pt");
	} else { /* echo */
		snprintf(req, sizeof(req), "IGKM-H-REQ-%s guest->peer line 1\n", tag);
		wr(fd, req);
		rd_plain(fd, "echo");
	}
	close(fd);
	printf("IGKMG-APP: DONE mode=%s\n", mode);
	return 0;
}
