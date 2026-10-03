// SPDX-License-Identifier: BSD-3-Clause
/*
 * Spike E (GH #303, increment-j) guest application. THROWAWAY.
 * An UNMODIFIED-style Linux app: plain BSD sockets, no TLS awareness. The
 * in-guest kernel module transparently mTLS-wraps mesh connections via kTLS.
 *
 * Regression modes (Spike C/D, item 6): echo / passthrough / lbfresh / deny.
 * New Spike E modes:
 *   server   <bindport> <nconn>      blocking accept server (Part 1): accept
 *                                    nconn inbound conns; each gets server-side
 *                                    kTLS; read one request line (plaintext in
 *                                    the guest), write a byte-distinct response.
 *   nbclient <vip> <port> <sub>      non-blocking connect + epoll (Part 2):
 *                                    sub=ok  -> EINPROGRESS, pre-ready write
 *                                               must EAGAIN, EPOLLOUT only after
 *                                               kTLS, then byte-exact exchange.
 *                                    sub=hsdeny -> EINPROGRESS then EPOLLERR,
 *                                               SO_ERROR=EACCES, no app bytes.
 *                                    sub=rdeny  -> connect() itself EACCES
 *                                               (resolve-time deny, no TCP).
 *   nbserver <bindport> <nbytes>     non-blocking accept + edge-triggered epoll
 *                                    (Part 2): accept4(NONBLOCK) one inbound
 *                                    conn; read nbytes via ET small reads (every
 *                                    byte, one rising edge per record); reply.
 */
#define _GNU_SOURCE	/* accept4(), SOCK_NONBLOCK, EPOLLET */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <time.h>
#include <fcntl.h>
#include <sys/time.h>
#include <sys/socket.h>
#include <sys/epoll.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <arpa/inet.h>

static double g_t0;
static double mono(void)
{
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return ts.tv_sec + ts.tv_nsec / 1e9;
}
static const char *tp(void)
{
	static char b[40];
	snprintf(b, sizeof(b), "t+%.3fs", mono() - g_t0);
	return b;
}

/* ---------- Spike C/D regression (blocking client) ------------------------ */
static int connect_to(const char *ip, int port)
{
	int fd = socket(AF_INET, SOCK_STREAM, 0);
	struct sockaddr_in sin;
	struct timeval tv = { .tv_sec = 3, .tv_usec = 0 };

	if (fd < 0) { printf("IGKME-APP: socket errno=%d\n", errno); return -1; }
	setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
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
static void rd_plain(int fd, const char *what)
{
	char buf[2048];
	ssize_t n = read(fd, buf, sizeof(buf) - 1);

	if (n < 0) { printf("IGKMD-APP: %s read FAILED errno=%d (%s)\n", what, errno, strerror(errno)); return; }
	if (n == 0) { printf("IGKMD-APP: %s read EOF\n", what); return; }
	buf[n] = 0;
	printf("IGKMD-APP: %s read(%zd) PLAINTEXT: %s%s", what, n, buf, (buf[n-1] == '\n') ? "" : "\n");
}

static int regression(const char *mode, const char *ip, int port, const char *tag)
{
	char req[160];
	int fd;

	if (!strcmp(mode, "deny")) {
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
	} else {
		snprintf(req, sizeof(req), "IGKM-D-REQ-%s guest->peer line 1\n", tag);
		wr(fd, req);
		rd_plain(fd, "echo");
	}
	close(fd);
	printf("IGKMD-APP: DONE mode=%s\n", mode);
	return 0;
}

/* ---------- Part 1: blocking accept() server ------------------------------ */
static int mode_server(int bindport, int nconn)
{
	int ls = socket(AF_INET, SOCK_STREAM, 0), i;
	struct sockaddr_in sin;
	int one = 1;

	if (ls < 0) { printf("IGKME-APP: server socket errno=%d\n", errno); return 1; }
	setsockopt(ls, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
	memset(&sin, 0, sizeof(sin));
	sin.sin_family = AF_INET;
	sin.sin_addr.s_addr = htonl(INADDR_ANY);
	sin.sin_port = htons(bindport);
	if (bind(ls, (struct sockaddr *)&sin, sizeof(sin)) < 0) { printf("IGKME-APP: server bind :%d errno=%d\n", bindport, errno); return 1; }
	if (listen(ls, 8) < 0) { printf("IGKME-APP: server listen errno=%d\n", errno); return 1; }
	printf("IGKME-APP: SERVER-LISTENING-%d blocking accept, nconn=%d %s\n", bindport, nconn, tp());
	for (i = 0; i < nconn; i++) {
		struct sockaddr_in cli;
		socklen_t cl = sizeof(cli);
		struct timeval tv = { .tv_sec = 15, .tv_usec = 0 };
		char buf[2048], resp[2200];
		ssize_t n;
		int c = accept(ls, (struct sockaddr *)&cli, &cl);

		if (c < 0) { printf("IGKME-APP: server accept#%d FAILED errno=%d (%s) %s\n", i + 1, errno, strerror(errno), tp()); continue; }
		setsockopt(c, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
		printf("IGKME-APP: server accept#%d from %s:%d fd=%d (module installs server-side kTLS on first I/O) %s\n",
		       i + 1, inet_ntoa(cli.sin_addr), ntohs(cli.sin_port), c, tp());
		n = read(c, buf, sizeof(buf) - 1);
		if (n < 0) {
			printf("IGKME-APP: server conn#%d read FAILED errno=%d (%s) -- DENY expected here: no application bytes crossed %s\n",
			       i + 1, errno, strerror(errno), tp());
			close(c);
			continue;
		}
		if (n == 0) {
			printf("IGKME-APP: server conn#%d read EOF (handshake likely denied; no app bytes) %s\n", i + 1, tp());
			close(c);
			continue;
		}
		buf[n] = 0;
		printf("IGKME-APP: server conn#%d read(%zd) PLAINTEXT-IN-GUEST: %s%s", i + 1, n, buf, (buf[n-1] == '\n') ? "" : "\n");
		snprintf(resp, sizeof(resp), "IGKM-E-RESP from-guest-server conn%d echo[%.*s]\n", i + 1, (int)(n > 60 ? 60 : n), buf);
		if (write(c, resp, strlen(resp)) < 0)
			printf("IGKME-APP: server conn#%d write FAILED errno=%d %s\n", i + 1, errno, tp());
		else
			printf("IGKME-APP: server conn#%d wrote response (%zu bytes, encrypted by kTLS on the wire) %s\n", i + 1, strlen(resp), tp());
		close(c);
	}
	close(ls);
	printf("IGKME-APP: DONE mode=server %s\n", tp());
	return 0;
}

/* ---------- Part 2: non-blocking connect + epoll client ------------------- */
static int mode_nbclient(const char *ip, int port, const char *sub)
{
	int fd = socket(AF_INET, SOCK_STREAM, 0);
	struct sockaddr_in sin;
	int ep, fl, rc, soerr = 0, wrote = 0;
	socklen_t sl = sizeof(soerr);
	struct epoll_event ev, out[4];
	char req[128], buf[2048];
	int ready_seen = 0, wakes = 0;

	if (fd < 0) { printf("IGKME-APP: nbclient socket errno=%d\n", errno); return 1; }
	fl = fcntl(fd, F_GETFL, 0);
	fcntl(fd, F_SETFL, fl | O_NONBLOCK);
	memset(&sin, 0, sizeof(sin));
	sin.sin_family = AF_INET;
	sin.sin_port = htons(port);
	inet_pton(AF_INET, ip, &sin.sin_addr);

	printf("IGKME-APP: nbclient START vip=%s:%d sub=%s (O_NONBLOCK) %s\n", ip, port, sub, tp());
	rc = connect(fd, (struct sockaddr *)&sin, sizeof(sin));
	printf("IGKME-APP: nbclient connect() rc=%d errno=%d (%s) %s\n", rc, errno, strerror(errno), tp());
	if (rc == 0) {
		printf("IGKME-APP: nbclient UNEXPECTED immediate connect success (expected EINPROGRESS)\n");
	} else if (errno == EACCES) {
		/* resolve-time deny (Shape 2): connect() itself fails, no TCP SYN. */
		printf("IGKME-APP: nbclient RESOLVE-DENY: connect() returned EACCES=13 at resolve time; no TCP, no app bytes %s\n", tp());
		close(fd);
		return 0;
	} else if (errno != EINPROGRESS) {
		printf("IGKME-APP: nbclient connect() unexpected errno=%d %s\n", errno, tp());
		close(fd);
		return 1;
	}

	/* Prove the socket is NOT writable before kTLS: a write now must EAGAIN. */
	snprintf(req, sizeof(req), "IGKM-E-REQN nbclient->peer first line\n");
	rc = write(fd, req, strlen(req));
	printf("IGKME-APP: nbclient pre-ready write() rc=%d errno=%d (%s) [EAGAIN=11 expected: not writable before kTLS] %s\n",
	       rc, errno, strerror(errno), tp());

	ep = epoll_create1(0);
	ev.events = EPOLLOUT | EPOLLERR | EPOLLHUP;
	ev.data.fd = fd;
	epoll_ctl(ep, EPOLL_CTL_ADD, fd, &ev);

	while (!ready_seen && wakes < 200) {
		int n = epoll_wait(ep, out, 4, 15000);

		wakes++;
		if (n <= 0) { printf("IGKME-APP: nbclient epoll_wait rc=%d (timeout/err) %s\n", n, tp()); break; }
		printf("IGKME-APP: nbclient epoll wake#%d revents=0x%x %s\n", wakes, out[0].events, tp());
		if (out[0].events & (EPOLLERR | EPOLLHUP)) {
			getsockopt(fd, SOL_SOCKET, SO_ERROR, &soerr, &sl);
			printf("IGKME-APP: nbclient EPOLLERR -> SO_ERROR=%d (%s) [13=EACCES handshake-deny]; app bytes written=%d %s\n",
			       soerr, strerror(soerr), wrote, tp());
			close(ep); close(fd);
			return 0;
		}
		if (out[0].events & EPOLLOUT) {
			ready_seen = 1;
			printf("IGKME-APP: nbclient EPOLLOUT (writable) FIRST seen at wake#%d %s -- this is AFTER the module's kTLS install line above\n", wakes, tp());
			rc = write(fd, req, strlen(req));
			wrote = (rc > 0) ? rc : 0;
			printf("IGKME-APP: nbclient post-ready write() rc=%d errno=%d: %.*s%s", rc, errno, (int)strlen(req), req, tp());
		}
	}
	if (!ready_seen) { printf("IGKME-APP: nbclient never became writable %s\n", tp()); close(ep); close(fd); return 1; }

	/* read the byte-distinct response (now level-trigger for EPOLLIN) */
	ev.events = EPOLLIN | EPOLLERR | EPOLLHUP;
	epoll_ctl(ep, EPOLL_CTL_MOD, fd, &ev);
	{
		int n = epoll_wait(ep, out, 4, 15000);
		ssize_t r;

		if (n > 0 && (out[0].events & EPOLLIN)) {
			r = read(fd, buf, sizeof(buf) - 1);
			if (r > 0) { buf[r] = 0; printf("IGKME-APP: nbclient read(%zd) PLAINTEXT: %s%s", r, buf, (buf[r-1] == '\n') ? "" : "\n"); }
			else printf("IGKME-APP: nbclient read rc=%zd errno=%d %s\n", r, errno, tp());
		} else {
			printf("IGKME-APP: nbclient response epoll rc=%d revents=0x%x %s\n", n, n > 0 ? out[0].events : 0, tp());
		}
	}
	close(ep); close(fd);
	printf("IGKME-APP: DONE mode=nbclient sub=%s %s\n", sub, tp());
	return 0;
}

/* ---------- Part 2: non-blocking accept + edge-triggered epoll server ----- */
static int mode_nbserver(int bindport, long nbytes)
{
	int ls = socket(AF_INET, SOCK_STREAM, 0), ep, c = -1, fl;
	struct sockaddr_in sin;
	int one = 1, wakes = 0, edges_with_data = 0;
	long total = 0;
	unsigned long sum = 0;
	struct epoll_event ev, out[4];

	if (ls < 0) { printf("IGKME-APP: nbserver socket errno=%d\n", errno); return 1; }
	setsockopt(ls, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
	fl = fcntl(ls, F_GETFL, 0);
	fcntl(ls, F_SETFL, fl | O_NONBLOCK);
	memset(&sin, 0, sizeof(sin));
	sin.sin_family = AF_INET;
	sin.sin_addr.s_addr = htonl(INADDR_ANY);
	sin.sin_port = htons(bindport);
	if (bind(ls, (struct sockaddr *)&sin, sizeof(sin)) < 0) { printf("IGKME-APP: nbserver bind errno=%d\n", errno); return 1; }
	if (listen(ls, 8) < 0) { printf("IGKME-APP: nbserver listen errno=%d\n", errno); return 1; }

	ep = epoll_create1(0);
	ev.events = EPOLLIN;	/* listener: level-triggered accept readiness */
	ev.data.fd = ls;
	epoll_ctl(ep, EPOLL_CTL_ADD, ls, &ev);
	printf("IGKME-APP: SERVER-LISTENING-%d nonblocking+ET, expect %ld bytes %s\n", bindport, nbytes, tp());

	while (total < nbytes && wakes < 100000) {
		int n = epoll_wait(ep, out, 4, 20000), k;

		wakes++;
		if (n <= 0) { printf("IGKME-APP: nbserver epoll_wait rc=%d (timeout) total=%ld %s\n", n, total, tp()); break; }
		for (k = 0; k < n; k++) {
			if (out[k].data.fd == ls && (out[k].events & EPOLLIN)) {
				struct sockaddr_in cli;
				socklen_t cl = sizeof(cli);

				c = accept4(ls, (struct sockaddr *)&cli, &cl, SOCK_NONBLOCK);
				if (c < 0) { printf("IGKME-APP: nbserver accept4 errno=%d %s\n", errno, tp()); continue; }
				printf("IGKME-APP: nbserver accept4(NONBLOCK) child fd=%d from %s:%d; adding to epoll EPOLLIN|EPOLLET %s\n",
				       c, inet_ntoa(cli.sin_addr), ntohs(cli.sin_port), tp());
				ev.events = EPOLLIN | EPOLLET;	/* child: edge-triggered */
				ev.data.fd = c;
				epoll_ctl(ep, EPOLL_CTL_ADD, c, &ev);
			} else if (c >= 0 && out[k].data.fd == c) {
				if (out[k].events & (EPOLLERR | EPOLLHUP)) {
					int soerr = 0; socklen_t sl = sizeof(soerr);

					getsockopt(c, SOL_SOCKET, SO_ERROR, &soerr, &sl);
					printf("IGKME-APP: nbserver child EPOLLERR/HUP SO_ERROR=%d (%s) total=%ld %s\n", soerr, strerror(soerr), total, tp());
					goto done;
				}
				/* edge-triggered: drain with SMALL reads until EAGAIN */
				{
					long before = total;
					int reads = 0;
					char small[64];
					ssize_t r;

					for (;;) {
						r = read(c, small, sizeof(small));
						if (r > 0) {
							long i;

							for (i = 0; i < r; i++) sum = sum * 131 + (unsigned char)small[i];
							total += r; reads++;
							continue;
						}
						if (r == 0) { printf("IGKME-APP: nbserver child EOF total=%ld %s\n", total, tp()); goto reply; }
						if (errno == EAGAIN || errno == EWOULDBLOCK) break;
						printf("IGKME-APP: nbserver child read errno=%d (%s) total=%ld %s\n", errno, strerror(errno), total, tp());
						goto done;
					}
					edges_with_data++;
					printf("IGKME-APP: nbserver ET edge#%d drained %ld bytes in %d small reads (total=%ld/%ld) %s\n",
					       edges_with_data, total - before, reads, total, nbytes, tp());
				}
			}
		}
	}
reply:
	printf("IGKME-APP: nbserver RECEIVED total=%ld bytes over %d ET edges, rolling-hash=0x%lx (expected %ld) match=%d %s\n",
	       total, edges_with_data, sum, nbytes, total == nbytes, tp());
	if (c >= 0) {
		char resp[128];

		snprintf(resp, sizeof(resp), "IGKM-E-RESP nbserver got=%ld edges=%d hash=0x%lx\n", total, edges_with_data, sum);
		if (write(c, resp, strlen(resp)) > 0)
			printf("IGKME-APP: nbserver wrote ack (encrypted by kTLS) %s\n", tp());
		close(c);
	}
done:
	close(ep); close(ls);
	printf("IGKME-APP: DONE mode=nbserver %s\n", tp());
	return 0;
}

int main(int argc, char **argv)
{
	const char *mode = argc > 1 ? argv[1] : "echo";

	g_t0 = mono();
	setvbuf(stdout, NULL, _IONBF, 0);

	if (!strcmp(mode, "server"))
		return mode_server(argc > 2 ? atoi(argv[2]) : 8443, argc > 3 ? atoi(argv[3]) : 1);
	if (!strcmp(mode, "nbclient"))
		return mode_nbclient(argc > 2 ? argv[2] : "10.80.0.1", argc > 3 ? atoi(argv[3]) : 9443, argc > 4 ? argv[4] : "ok");
	if (!strcmp(mode, "nbserver"))
		return mode_nbserver(argc > 2 ? atoi(argv[2]) : 8443, argc > 3 ? atol(argv[3]) : 40000);

	/* regression (Spike C/D) */
	{
		const char *ip = argc > 2 ? argv[2] : "10.80.0.1";
		int port = argc > 3 ? atoi(argv[3]) : 9443;
		const char *tag = argc > 4 ? argv[4] : "1";

		printf("IGKMD-APP: START mode=%s vip=%s:%d tag=%s\n", mode, ip, port, tag);
		return regression(mode, ip, port, tag);
	}
}
