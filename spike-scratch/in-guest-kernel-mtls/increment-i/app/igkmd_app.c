// SPDX-License-Identifier: BSD-3-Clause
/*
 * Spike D (GH #303, increment-i) guest application. THROWAWAY.
 * An UNMODIFIED-style Linux app: plain blocking TCP sockets, no TLS awareness,
 * no service-discovery awareness. It connects to a stable service VIP by its
 * address; the in-guest kernel module resolves that VIP to an agent-chosen
 * backend over vsock, rewrites the connect destination, and transparently
 * mTLS-wraps the connection via kTLS. This program never sees keys, records,
 * resolution requests, or the backend address it actually landed on — except
 * that the backend's echo embeds `from-<label>`, so a plain read() reveals it.
 *
 *   igkmd_app <mode> <ip> <port> [tag]
 *     echo        connect VIP; write one REQ; one plain read(); print (lands on first-healthy)
 *     lbfresh     connect VIP; REQ/read (from-A); pause (host marks A unhealthy);
 *                 reconnect VIP; REQ/read (from-B) -- proves connect-time re-resolution
 *     deny        connect VIP of a policy-denied service; expect connect() EACCES, no TCP
 *     passthrough connect; REQ/read (plain, non-mesh positive control)
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

static int connect_to(const char *ip, int port)
{
	int fd = socket(AF_INET, SOCK_STREAM, 0);
	struct sockaddr_in sin;
	struct timeval tv = { .tv_sec = 3, .tv_usec = 0 };

	if (fd < 0) { printf("IGKMD-APP: socket errno=%d\n", errno); return -1; }
	setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv)); /* bound reads so a stall can't hang the run */
	memset(&sin, 0, sizeof(sin));
	sin.sin_family = AF_INET;
	sin.sin_port = htons(port);
	inet_pton(AF_INET, ip, &sin.sin_addr);
	if (connect(fd, (struct sockaddr *)&sin, sizeof(sin)) < 0) {
		printf("IGKMD-APP: connect VIP %s:%d FAILED errno=%d (%s)\n", ip, port, errno, strerror(errno));
		close(fd);
		return -1;
	}
	printf("IGKMD-APP: connect VIP %s:%d returned 0 (module resolved+rewrote dst and installed transparent mTLS if mesh)\n", ip, port);
	return fd;
}

static void wr(int fd, const char *s)
{
	ssize_t n = write(fd, s, strlen(s));

	printf("IGKMD-APP: write(%zu) rc=%zd: %.*s", strlen(s), n, (int)strlen(s), s);
}

/* plain read(): the unmodified-app behaviour. The response embeds from-<label>. */
static void rd_plain(int fd, const char *what)
{
	char buf[2048];
	ssize_t n = read(fd, buf, sizeof(buf) - 1);

	if (n < 0) { printf("IGKMD-APP: %s read FAILED errno=%d (%s)\n", what, errno, strerror(errno)); return; }
	if (n == 0) { printf("IGKMD-APP: %s read EOF\n", what); return; }
	buf[n] = 0;
	printf("IGKMD-APP: %s read(%zd) PLAINTEXT: %s%s", what, n, buf, (buf[n-1] == '\n') ? "" : "\n");
}

int main(int argc, char **argv)
{
	const char *mode = argc > 1 ? argv[1] : "echo";
	const char *ip = argc > 2 ? argv[2] : "10.80.0.1";
	int port = argc > 3 ? atoi(argv[3]) : 9443;
	const char *tag = argc > 4 ? argv[4] : "1";
	char req[160];
	int fd;

	setvbuf(stdout, NULL, _IONBF, 0);
	printf("IGKMD-APP: START mode=%s vip=%s:%d tag=%s\n", mode, ip, port, tag);

	if (!strcmp(mode, "deny")) {
		/* Shape 2: the module resolves the VIP under the connect() lock; a
		 * policy-denied service returns DENY, so connect() itself fails with
		 * EACCES and no TCP SYN is ever sent. */
		fd = connect_to(ip, port);
		if (fd < 0) {
			printf("IGKMD-APP: DENY-CASE OK: connect() refused (expect EACCES=13); no TCP SYN, no application bytes on the wire\n");
			return 0;
		}
		printf("IGKMD-APP: DENY-CASE UNEXPECTED: connect() to a denied service succeeded fd=%d\n", fd);
		close(fd);
		return 1;
	}

	if (!strcmp(mode, "lbfresh")) {
		fd = connect_to(ip, port);
		if (fd < 0) { printf("IGKMD-APP: lbfresh connect-1 FAILED\n"); return 1; }
		snprintf(req, sizeof(req), "IGKM-D-REQ-%s-c1 guest->peer lb-connect-1\n", tag);
		wr(fd, req);
		rd_plain(fd, "lb-c1");
		close(fd);
		printf("IGKMD-APP: PAUSE-FOR-HEALTH-TOGGLE (sleeping 3s; host marks first-choice backend A unhealthy now)\n");
		sleep(3);
		fd = connect_to(ip, port);
		if (fd < 0) { printf("IGKMD-APP: lbfresh connect-2 FAILED\n"); return 1; }
		snprintf(req, sizeof(req), "IGKM-D-REQ-%s-c2 guest->peer lb-connect-2-after-health-toggle\n", tag);
		wr(fd, req);
		rd_plain(fd, "lb-c2");
		close(fd);
		/* Hold the VM alive briefly so the tap capture flushes connect-2's full
		 * exchange + close to the pcap BEFORE init's hard reboot tears the NIC
		 * down (otherwise the tail of B's traffic is lost from the capture). */
		printf("IGKMD-APP: lbfresh settle (sleeping 2s so the capture flushes)\n");
		sleep(2);
		printf("IGKMD-APP: DONE mode=lbfresh\n");
		return 0;
	}

	fd = connect_to(ip, port);
	if (fd < 0) return 1;

	if (!strcmp(mode, "passthrough")) {
		snprintf(req, sizeof(req), "IGKM-D-REQ-PT-%s guest->peer plaintext\n", tag);
		wr(fd, req);
		rd_plain(fd, "pt");
	} else { /* echo */
		snprintf(req, sizeof(req), "IGKM-D-REQ-%s guest->peer line 1\n", tag);
		wr(fd, req);
		rd_plain(fd, "echo");
	}
	close(fd);
	printf("IGKMD-APP: DONE mode=%s\n", mode);
	return 0;
}
