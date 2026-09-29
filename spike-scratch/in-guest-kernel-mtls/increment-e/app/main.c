/*
 * Spike B2 guest program (GH #303 in-guest-kernel-mtls, increment-e).
 *
 * An ORDINARY program using plain TCP sockets: blocking, non-blocking with
 * epoll (level- and edge-triggered) and non-blocking with poll(). It contains
 * no TLS, no certificates and no knowledge of the mesh. Whether a connection
 * is encrypted is decided underneath it, by lib-mtlsguard and the host relay.
 *
 * argv[1] = "run" (main checklist) or "stretch" (non-blocking accept).
 *
 * run:
 *   A  blocking pass-through         :5001 plain
 *   B  blocking deny                 :6444 -> connect() = -1 EACCES
 *   D  blocking server-first         :6445 (peer also sends 2 NewSessionTickets)
 *   S  blocking accept on :7443      S1 allowed caller, S2 denied caller
 *   N1 non-blocking + epoll LT       :6443 client-first; EINPROGRESS, no EPOLLOUT
 *                                    and write() = EAGAIN until the record layer
 *                                    is installed; 2 NSTs must not raise EPOLLIN
 *   N2 non-blocking + epoll LT       :6445 server-first
 *   N3 non-blocking + epoll ET       :6447 1-byte reads of a response line plus a
 *                                    40000-byte multi-record bulk payload
 *   N4 non-blocking + poll() LT      :6445 server-first, ONE byte per poll wakeup
 *   K  non-blocking + epoll LT       :6446 3 exchanges, each response preceded by
 *                                    a peer KeyUpdate(update_requested)
 *   R1 non-blocking + epoll LT       :6448 crafted NSTs (split + coalesced with
 *                                    app data) and a KeyUpdate(update_not_requested)
 *   R2 non-blocking + epoll LT       :6449 an unexpected post-handshake message
 *                                    -> read() = -1 EPROTO, data after it never read
 *   N5 non-blocking deny             :6444 -> EPOLLERR|EPOLLHUP, SO_ERROR = EACCES
 *   E  blocking client-first         :6450 + host relay killed + 2nd exchange
 *   F  blocking agent-down           :6443 -> ECONNABORTED
 *   N6 non-blocking agent-down       :6443 -> EPOLLERR, SO_ERROR = ECONNABORTED
 * stretch:
 *   SN non-blocking listener under epoll: accept4() must return at once; the
 *      allowed caller's request arrives, the denied caller's fd reports an error.
 */
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/epoll.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <uk/plat/time.h>

#define HOST_U32 0xC0A8CC01u /* 192.168.204.1 */
#define KILL_WAIT_S 8
#define BULK_LEN 40000
#define HS_WAIT_MS 10000
#define IO_WAIT_MS 5000

static const char *mono(void)
{
	static char buf[32];
	unsigned long long ns = (unsigned long long)ukplat_monotonic_clock();

	snprintf(buf, sizeof(buf), "%llu.%06llu", ns / 1000000000ull,
		 (ns % 1000000000ull) / 1000ull);
	return buf;
}

static unsigned long long now_ns(void)
{
	return (unsigned long long)ukplat_monotonic_clock();
}

static const char *evs(unsigned int e)
{
	static char b[64];

	snprintf(b, sizeof(b), "%s%s%s%s%s", e ? "" : "none ",
		 (e & EPOLLIN) ? "IN " : "", (e & EPOLLOUT) ? "OUT " : "",
		 (e & EPOLLERR) ? "ERR " : "", (e & EPOLLHUP) ? "HUP " : "");
	return b;
}

static int write_all(int fd, const char *buf, size_t len)
{
	while (len > 0) {
		ssize_t n = write(fd, buf, len);

		if (n < 0) {
			if (errno == EINTR)
				continue;
			return -1;
		}
		buf += n;
		len -= (size_t)n;
	}
	return 0;
}

static long read_line(int fd, char *buf, size_t cap)
{
	size_t got = 0;

	while (got + 1 < cap) {
		ssize_t n = read(fd, buf + got, cap - 1 - got);

		if (n < 0) {
			if (errno == EINTR)
				continue;
			return -1;
		}
		if (n == 0)
			break;
		got += (size_t)n;
		if (memchr(buf, '\n', got))
			break;
	}
	buf[got] = '\0';
	return (long)got;
}

static struct sockaddr_in host_addr(int port)
{
	struct sockaddr_in sa;

	memset(&sa, 0, sizeof(sa));
	sa.sin_family = AF_INET;
	sa.sin_port = htons(port);
	sa.sin_addr.s_addr = htonl(HOST_U32);
	return sa;
}

/* ---- blocking helpers (Spike B) ------------------------------------------ */

static int open_to(const char *c, int port, int *err)
{
	struct sockaddr_in sa = host_addr(port);
	unsigned long long t0;
	int fd, rc;

	fd = socket(AF_INET, SOCK_STREAM, 0);
	if (fd < 0) {
		*err = errno;
		return -1;
	}
	printf("IGKM-E: %s blocking connect(192.168.204.1:%d) t=%s\n", c, port,
	       mono());
	t0 = now_ns();
	rc = connect(fd, (struct sockaddr *)&sa, sizeof(sa));
	*err = rc ? errno : 0;
	printf("IGKM-E: %s connect() returned %d errno=%d (%s) after %llu us t=%s\n",
	       c, rc, *err, *err ? strerror(*err) : "-",
	       (now_ns() - t0) / 1000ull, mono());
	if (rc) {
		close(fd);
		return -1;
	}
	return fd;
}

static int send_line(const char *c, int fd, const char *line)
{
	if (write_all(fd, line, strlen(line)) < 0) {
		printf("IGKM-E: %s write FAILED errno=%d (%s)\n", c, errno,
		       strerror(errno));
		return -1;
	}
	printf("IGKM-E: %s wrote %lu bytes: %s", c,
	       (unsigned long)strlen(line), line);
	return 0;
}

static int expect_line(const char *c, int fd, const char *want)
{
	char buf[256];
	long n = read_line(fd, buf, sizeof(buf));

	if (n < 0) {
		printf("IGKM-E: %s read FAILED errno=%d (%s)\n", c, errno,
		       strerror(errno));
		return -1;
	}
	printf("IGKM-E: %s read %ld bytes: %s%s", c, n, buf,
	       (n == 0 || buf[n - 1] != '\n') ? "\n" : "");
	if ((size_t)n != strlen(want) || memcmp(buf, want, (size_t)n)) {
		printf("IGKM-E: %s MISMATCH (want %s)", c, want);
		return -1;
	}
	return 0;
}

/* ---- non-blocking helpers ------------------------------------------------ */

struct nb {
	const char *c;
	int fd, ep;
	unsigned long long t_connect_ret, t_out;
	unsigned int empty_waits, early_eagain, early_other, spurious_hs;
	unsigned int in_wakeups, spurious_in;
	unsigned int last_ev;
};

static const char EARLY[] = "IGKM-E-EARLY must-not-leave guest->peer\n";

/* Try the application's first write early; it must be refused with EAGAIN. */
static void early_write(struct nb *s)
{
	ssize_t w = write(s->fd, EARLY, sizeof(EARLY) - 1);

	if (w < 0 && errno == EAGAIN) {
		s->early_eagain++;
	} else {
		s->early_other++;
		printf("IGKM-E: %s early write() returned %ld errno=%d (%s) -> UNEXPECTED\n",
		       s->c, (long)w, w < 0 ? errno : 0,
		       w < 0 ? strerror(errno) : "-");
	}
}

/* socket(SOCK_NONBLOCK) + connect(): must be -1 EINPROGRESS. */
static int nb_connect(struct nb *s, const char *c, int port)
{
	struct sockaddr_in sa = host_addr(port);
	struct pollfd pfd;
	unsigned long long t0;
	int rc, e;
	ssize_t w;

	memset(s, 0, sizeof(*s));
	s->c = c;
	s->ep = -1;
	s->fd = socket(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0);
	if (s->fd < 0)
		return -1;
	printf("IGKM-E: %s non-blocking connect(192.168.204.1:%d) t=%s\n", c,
	       port, mono());
	t0 = now_ns();
	rc = connect(s->fd, (struct sockaddr *)&sa, sizeof(sa));
	e = rc ? errno : 0;
	s->t_connect_ret = now_ns();
	printf("IGKM-E: %s connect() returned %d errno=%d (%s) after %llu us t=%s\n",
	       c, rc, e, e ? strerror(e) : "-",
	       (s->t_connect_ret - t0) / 1000ull, mono());
	if (!(rc == -1 && e == EINPROGRESS)) {
		printf("IGKM-E: %s expected -1 EINPROGRESS\n", c);
		return -1;
	}
	pfd.fd = s->fd;
	pfd.events = POLLIN | POLLOUT;
	pfd.revents = 0;
	rc = poll(&pfd, 1, 0);
	printf("IGKM-E: %s poll(POLLIN|POLLOUT, 0 ms) right after connect() = %d revents=0x%x t=%s\n",
	       c, rc, pfd.revents, mono());
	w = write(s->fd, EARLY, sizeof(EARLY) - 1);
	e = w < 0 ? errno : 0;
	printf("IGKM-E: %s write(%lu bytes) before EPOLLOUT returned %ld errno=%d (%s) t=%s\n",
	       c, (unsigned long)(sizeof(EARLY) - 1), (long)w, e,
	       e ? strerror(e) : "-", mono());
	if (rc != 0 || !(w < 0 && e == EAGAIN)) {
		printf("IGKM-E: %s expected poll()=0 and write()=-1 EAGAIN before the handshake\n",
		       c);
		return -1;
	}
	s->early_eagain = 1;
	return 0;
}

/* Wait (epoll) for EPOLLOUT or an error; probe every 1 ms, and every fourth
 * empty probe retry the early write. */
static int nb_wait_out_epoll(struct nb *s, int et)
{
	struct epoll_event ev = { .events = EPOLLIN | EPOLLOUT |
					    (et ? EPOLLET : 0),
				  .data.fd = s->fd };
	unsigned long long deadline = now_ns() + HS_WAIT_MS * 1000000ull;

	s->ep = epoll_create1(0);
	if (s->ep < 0 || epoll_ctl(s->ep, EPOLL_CTL_ADD, s->fd, &ev))
		return -1;
	while (now_ns() < deadline) {
		struct epoll_event out[4];
		int n = epoll_wait(s->ep, out, 4, 1);

		if (n < 0)
			return -1;
		if (n == 0) {
			if (++s->empty_waits % 4 == 0)
				early_write(s);
			continue;
		}
		s->last_ev = out[0].events;
		printf("IGKM-E: %s epoll_wait -> %s(%s) after %u empty 1-ms probes t=%s\n",
		       s->c, evs(out[0].events), et ? "ET" : "LT",
		       s->empty_waits, mono());
		if (out[0].events & (EPOLLERR | EPOLLHUP))
			return 1; /* error: caller checks SO_ERROR */
		if (out[0].events & EPOLLOUT) {
			s->t_out = now_ns();
			break;
		}
		s->spurious_hs++; /* EPOLLIN without EPOLLOUT during handshake */
	}
	if (!s->t_out)
		return -1;
	printf("IGKM-E: %s EPOLLOUT observed %llu us after connect() returned; during the wait: %u empty epoll_wait probes, early write() -> EAGAIN x%u, other results x%u, EPOLLIN-before-EPOLLOUT x%u t=%s\n",
	       s->c, (s->t_out - s->t_connect_ret) / 1000ull, s->empty_waits,
	       s->early_eagain, s->early_other, s->spurious_hs, mono());
	return (s->early_other || s->spurious_hs) ? -1 : 0;
}

static int so_error(int fd)
{
	int e = -1;
	socklen_t l = sizeof(e);

	if (getsockopt(fd, SOL_SOCKET, SO_ERROR, &e, &l))
		return -errno;
	return e;
}

/* Level-triggered epoll read of one line: on every EPOLLIN, read until
 * EAGAIN. A wakeup whose first read() is EAGAIN counts as spurious. */
static int epoll_read_line(struct nb *s, char *buf, size_t cap, long *outn)
{
	struct epoll_event ev = { .events = EPOLLIN, .data.fd = s->fd };
	size_t got = 0;

	if (epoll_ctl(s->ep, EPOLL_CTL_MOD, s->fd, &ev))
		return -1;
	for (;;) {
		struct epoll_event out[4];
		int n = epoll_wait(s->ep, out, 4, IO_WAIT_MS), first = 1;

		if (n <= 0) {
			printf("IGKM-E: %s epoll_wait for EPOLLIN timed out (readiness lost?) got=%lu\n",
			       s->c, (unsigned long)got);
			return -1;
		}
		s->in_wakeups++;
		s->last_ev = out[0].events;
		for (;;) {
			ssize_t r = read(s->fd, buf + got, cap - 1 - got);

			if (r > 0) {
				got += (size_t)r;
				first = 0;
				continue;
			}
			if (r < 0 && errno == EAGAIN) {
				if (first)
					s->spurious_in++;
				break;
			}
			buf[got] = '\0';
			*outn = r < 0 ? -errno : (long)got;
			return r < 0 ? 1 : 0; /* error or EOF */
		}
		buf[got] = '\0';
		if (memchr(buf, '\n', got)) {
			*outn = (long)got;
			return 0;
		}
	}
}

static int nb_exchange(struct nb *s, const char *req, const char *want)
{
	char buf[512];
	long n = 0;
	int r;

	if (req && send_line(s->c, s->fd, req))
		return -1;
	r = epoll_read_line(s, buf, sizeof(buf), &n);
	if (r) {
		printf("IGKM-E: %s read ended %ld (%s) events=%s\n", s->c, n,
		       n < 0 ? strerror((int)-n) : "EOF", evs(s->last_ev));
		return -1;
	}
	printf("IGKM-E: %s read %ld bytes: %s", s->c, n, buf);
	if ((size_t)n != strlen(want) || memcmp(buf, want, (size_t)n)) {
		printf("IGKM-E: %s MISMATCH (want %s)", s->c, want);
		return -1;
	}
	return 0;
}

static void nb_close(struct nb *s)
{
	if (s->ep >= 0)
		close(s->ep);
	if (s->fd >= 0)
		close(s->fd);
	s->ep = s->fd = -1;
}

/* ---- cases ----------------------------------------------------------------- */

static int case_passthrough(void)
{
	int err, rc, fd = open_to("A pass-through", 5001, &err);

	if (fd < 0)
		return -1;
	rc = send_line("A pass-through", fd,
		       "IGKM-E-PLAIN-REQ pass-through guest->host\n");
	if (!rc)
		rc = expect_line("A pass-through", fd,
				 "IGKM-E-PLAIN-RESP pass-through host->guest\n");
	close(fd);
	return rc;
}

static int case_deny(void)
{
	int err, fd = open_to("B deny", 6444, &err);

	if (fd >= 0) {
		printf("IGKM-E: B deny: connect() SUCCEEDED (policy not enforced)\n");
		close(fd);
		return -1;
	}
	return err == EACCES ? 0 : -1;
}

static int case_server_first_blocking(void)
{
	int err, rc, fd = open_to("D server-first", 6445, &err);

	if (fd < 0)
		return -1;
	rc = expect_line("D server-first", fd,
			 "IGKM-E-GREETING server-first peer->guest\n");
	if (!rc)
		rc = send_line("D server-first", fd,
			       "IGKM-E-REQS server-first guest->peer\n");
	if (!rc)
		rc = expect_line("D server-first", fd,
				 "IGKM-E-RESPS server-first peer->guest\n");
	close(fd);
	return rc;
}

static int case_inbound(int *s2)
{
	struct sockaddr_in sa, peer;
	socklen_t pl = sizeof(peer);
	char buf[256];
	int lfd, fd, rc = -1, one = 1;
	long n;

	*s2 = -1;
	lfd = socket(AF_INET, SOCK_STREAM, 0);
	if (lfd < 0)
		return -1;
	setsockopt(lfd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
	memset(&sa, 0, sizeof(sa));
	sa.sin_family = AF_INET;
	sa.sin_port = htons(7443);
	sa.sin_addr.s_addr = htonl(INADDR_ANY);
	if (bind(lfd, (struct sockaddr *)&sa, sizeof(sa)) || listen(lfd, 4)) {
		printf("IGKM-E: S bind/listen FAILED errno=%d\n", errno);
		close(lfd);
		return -1;
	}
	printf("IGKM-E: S blocking listen on 0.0.0.0:7443 READY-FOR-INBOUND t=%s\n",
	       mono());
	fd = accept(lfd, (struct sockaddr *)&peer, &pl);
	printf("IGKM-E: S1 accept() returned %d errno=%d t=%s\n", fd,
	       fd < 0 ? errno : 0, mono());
	if (fd >= 0) {
		n = read_line(fd, buf, sizeof(buf));
		printf("IGKM-E: S1 read %ld bytes: %s%s", n, n > 0 ? buf : "",
		       (n <= 0 || buf[n - 1] != '\n') ? "\n" : "");
		if (n == (long)strlen("IGKM-E-REQI inbound peer->guest\n") &&
		    !memcmp(buf, "IGKM-E-REQI inbound peer->guest\n", (size_t)n))
			rc = send_line("S1 inbound", fd,
				       "IGKM-E-RESPI inbound guest->peer\n");
		close(fd);
	}
	pl = sizeof(peer);
	fd = accept(lfd, (struct sockaddr *)&peer, &pl);
	n = fd < 0 ? errno : 0;
	printf("IGKM-E: S2 accept() returned %d errno=%ld (%s) t=%s\n", fd, n,
	       n ? strerror((int)n) : "-", mono());
	if (fd >= 0)
		close(fd);
	else if (n == ECONNABORTED)
		*s2 = 0;
	close(lfd);
	return rc;
}

/* N1: epoll LT, client-first. */
static int case_n1(void)
{
	struct nb s;
	int rc;

	if (nb_connect(&s, "N1 epoll-LT client-first", 6443) ||
	    nb_wait_out_epoll(&s, 0)) {
		nb_close(&s);
		return -1;
	}
	printf("IGKM-E: N1 SO_ERROR after EPOLLOUT = %d\n", so_error(s.fd));
	rc = nb_exchange(&s, "IGKM-E-REQ1 nb-client-first guest->peer\n",
			 "IGKM-E-RESP1 nb-client-first peer->guest\n");
	printf("IGKM-E: N1 EPOLLIN wakeups=%u spurious (EPOLLIN but first read() = EAGAIN)=%u\n",
	       s.in_wakeups, s.spurious_in);
	nb_close(&s);
	return (rc || s.spurious_in) ? -1 : 0;
}

/* N2: epoll LT, server-first. */
static int case_n2(void)
{
	struct nb s;
	int rc;

	if (nb_connect(&s, "N2 epoll-LT server-first", 6445) ||
	    nb_wait_out_epoll(&s, 0)) {
		nb_close(&s);
		return -1;
	}
	rc = nb_exchange(&s, NULL, "IGKM-E-GREETING server-first peer->guest\n");
	if (!rc)
		rc = nb_exchange(&s, "IGKM-E-REQS server-first guest->peer\n",
				 "IGKM-E-RESPS server-first peer->guest\n");
	printf("IGKM-E: N2 EPOLLIN wakeups=%u spurious=%u\n", s.in_wakeups,
	       s.spurious_in);
	nb_close(&s);
	return (rc || s.spurious_in) ? -1 : 0;
}

static unsigned int fnv1a32(const unsigned char *b, size_t n)
{
	unsigned int h = 0x811c9dc5u;
	size_t i;

	for (i = 0; i < n; i++) {
		h ^= b[i];
		h *= 0x01000193u;
	}
	return h;
}

static unsigned char bulk_rx[BULK_LEN];

/* N3: epoll ET, the app reads ONE byte per read() until EAGAIN. */
static int case_n3(void)
{
	static const char req[] = "IGKM-E-REQB et-bulk guest->peer\n";
	static const char want[] = "IGKM-E-RESPB et-bulk peer->guest\n";
	char line[64];
	size_t lgot = 0, bgot = 0, wl = sizeof(want) - 1;
	unsigned long reads = 0, wakeups = 0, spurious = 0, bad = 0;
	struct nb s;
	unsigned long long t0;

	if (nb_connect(&s, "N3 epoll-ET 1-byte-reads", 6447) ||
	    nb_wait_out_epoll(&s, 1)) {
		nb_close(&s);
		return -1;
	}
	if (send_line(s.c, s.fd, req)) {
		nb_close(&s);
		return -1;
	}
	t0 = now_ns();
	while (lgot + bgot < wl + BULK_LEN) {
		struct epoll_event out[4];
		int n = epoll_wait(s.ep, out, 4, IO_WAIT_MS), first = 1;

		if (n <= 0) {
			printf("IGKM-E: N3 epoll_wait (ET) timed out with %lu of %lu bytes -> readiness LOST for buffered data\n",
			       (unsigned long)(lgot + bgot),
			       (unsigned long)(wl + BULK_LEN));
			nb_close(&s);
			return -1;
		}
		if (!(out[0].events & EPOLLIN))
			continue; /* e.g. an EPOLLOUT edge */
		wakeups++;
		for (;;) {
			unsigned char ch;
			ssize_t r = read(s.fd, &ch, 1);

			if (r == 1) {
				reads++;
				first = 0;
				if (lgot < wl)
					line[lgot++] = (char)ch;
				else if (bgot < BULK_LEN)
					bulk_rx[bgot++] = ch;
				continue;
			}
			if (r < 0 && errno == EAGAIN) {
				if (first)
					spurious++;
				break;
			}
			printf("IGKM-E: N3 read() = %ld errno=%d\n", (long)r,
			       r < 0 ? errno : 0);
			nb_close(&s);
			return -1;
		}
	}
	line[lgot] = '\0';
	for (size_t i = 0; i < BULK_LEN; i++)
		if (bulk_rx[i] != (unsigned char)((i * 7 + 3) % 251))
			bad++;
	printf("IGKM-E: N3 read %s", line);
	printf("IGKM-E: N3 received %lu bytes in %lu one-byte read()s over %lu EPOLLIN (ET) wakeups, spurious=%lu, in %llu ms; bulk %lu bytes fnv1a32=0x%08x, bytes differing from the pattern=%lu\n",
	       (unsigned long)(lgot + bgot), reads, wakeups, spurious,
	       (now_ns() - t0) / 1000000ull, (unsigned long)bgot,
	       fnv1a32(bulk_rx, bgot), bad);
	nb_close(&s);
	return (strcmp(line, want) || bad || spurious) ? -1 : 0;
}

/* N4: poll() LT, server-first, ONE byte per poll() wakeup. */
static int poll_read_line_1(int fd, const char *c, char *buf, size_t cap,
			    unsigned long *wakeups, unsigned long *spurious)
{
	size_t got = 0;

	while (got + 1 < cap) {
		struct pollfd p = { .fd = fd, .events = POLLIN };
		int n = poll(&p, 1, IO_WAIT_MS);
		ssize_t r;

		if (n <= 0) {
			printf("IGKM-E: %s poll(POLLIN) timed out at %lu bytes -> readiness LOST\n",
			       c, (unsigned long)got);
			return -1;
		}
		(*wakeups)++;
		r = read(fd, buf + got, 1);
		if (r == 1) {
			got++;
			if (buf[got - 1] == '\n')
				break;
			continue;
		}
		if (r < 0 && errno == EAGAIN) {
			(*spurious)++;
			continue;
		}
		return -1;
	}
	buf[got] = '\0';
	return (int)got;
}

static int case_n4(void)
{
	static const char g[] = "IGKM-E-GREETING server-first peer->guest\n";
	static const char req[] = "IGKM-E-REQP poll-1byte guest->peer\n";
	static const char want[] = "IGKM-E-RESPP poll-1byte peer->guest\n";
	unsigned long long deadline, t_conn;
	unsigned long wakeups = 0, spurious = 0, empty = 0, eagain = 0;
	char buf[256];
	struct nb s;
	int n, rc = -1;

	if (nb_connect(&s, "N4 poll 1-byte-per-wakeup", 6445)) {
		nb_close(&s);
		return -1;
	}
	t_conn = s.t_connect_ret;
	deadline = now_ns() + HS_WAIT_MS * 1000000ull;
	for (;;) {
		struct pollfd p = { .fd = s.fd, .events = POLLIN | POLLOUT };

		n = poll(&p, 1, 1);
		if (n < 0 || now_ns() > deadline)
			goto out;
		if (n == 0) {
			if (++empty % 4 == 0) {
				ssize_t w = write(s.fd, EARLY, sizeof(EARLY) - 1);

				if (w < 0 && errno == EAGAIN)
					eagain++;
				else
					goto out;
			}
			continue;
		}
		printf("IGKM-E: N4 poll -> revents=0x%x after %lu empty 1-ms polls, %llu us after connect(); early write() -> EAGAIN x%lu t=%s\n",
		       p.revents, empty, (now_ns() - t_conn) / 1000ull,
		       eagain + 1, mono());
		if (p.revents & (POLLERR | POLLHUP))
			goto out;
		if (p.revents & POLLOUT)
			break;
	}
	n = poll_read_line_1(s.fd, s.c, buf, sizeof(buf), &wakeups, &spurious);
	printf("IGKM-E: N4 read %d bytes one per poll() wakeup: %s", n, buf);
	if (n != (int)strlen(g) || memcmp(buf, g, (size_t)n))
		goto out;
	if (send_line(s.c, s.fd, req))
		goto out;
	n = poll_read_line_1(s.fd, s.c, buf, sizeof(buf), &wakeups, &spurious);
	printf("IGKM-E: N4 read %d bytes one per poll() wakeup: %s", n, buf);
	if (n != (int)strlen(want) || memcmp(buf, want, (size_t)n))
		goto out;
	printf("IGKM-E: N4 POLLIN wakeups=%lu for %lu bytes (level-triggered readiness kept while plaintext stays buffered), spurious=%lu\n",
	       wakeups, (unsigned long)(strlen(g) + strlen(want)), spurious);
	rc = spurious ? -1 : 0;
out:
	nb_close(&s);
	return rc;
}

/* K: three exchanges; the peer precedes every response with a KeyUpdate. */
static int case_keyupdate(void)
{
	struct nb s;
	char req[96], want[96];
	int i, rc = 0;

	if (nb_connect(&s, "K keyupdate", 6446) || nb_wait_out_epoll(&s, 0)) {
		nb_close(&s);
		return -1;
	}
	for (i = 1; i <= 3 && !rc; i++) {
		snprintf(req, sizeof(req),
			 "IGKM-E-REQK%d keyupdate guest->peer\n", i);
		snprintf(want, sizeof(want),
			 "IGKM-E-RESPK%d keyupdate peer->guest\n", i);
		rc = nb_exchange(&s, req, want);
	}
	printf("IGKM-E: K %d exchanges, EPOLLIN wakeups=%u spurious=%u\n",
	       i - 1, s.in_wakeups, s.spurious_in);
	nb_close(&s);
	return (rc || s.spurious_in) ? -1 : 0;
}

/* R1: crafted NewSessionTickets (split across records, two per record,
 * coalesced with app data) and a KeyUpdate(update_not_requested). */
static int case_raw_ok(void)
{
	struct nb s;
	int rc;

	if (nb_connect(&s, "R1 raw-nst", 6448) || nb_wait_out_epoll(&s, 0)) {
		nb_close(&s);
		return -1;
	}
	rc = nb_exchange(&s, NULL, "IGKM-E-GREETING raw-nst peer->guest\n");
	if (!rc)
		rc = nb_exchange(&s, "IGKM-E-REQR raw-nst guest->peer\n",
				 "IGKM-E-RESPR raw-nst peer->guest\n");
	if (!rc)
		rc = nb_exchange(&s, "IGKM-E-REQR2 after-keyupdate guest->peer\n",
				 "IGKM-E-RESPR2 after-keyupdate peer->guest\n");
	printf("IGKM-E: R1 EPOLLIN wakeups=%u spurious=%u\n", s.in_wakeups,
	       s.spurious_in);
	nb_close(&s);
	return (rc || s.spurious_in) ? -1 : 0;
}

/* R2: an unexpected post-handshake handshake message must fail closed. */
static int case_raw_bad(void)
{
	struct nb s;
	char buf[256];
	long n = 0;
	int r;

	if (nb_connect(&s, "R2 raw-unexpected", 6449) ||
	    nb_wait_out_epoll(&s, 0)) {
		nb_close(&s);
		return -1;
	}
	r = epoll_read_line(&s, buf, sizeof(buf), &n);
	printf("IGKM-E: R2 epoll events=%s read result=%d n=%ld (%s); application bytes delivered=%lu\n",
	       evs(s.last_ev), r, n, n < 0 ? strerror((int)-n) : "-",
	       (unsigned long)strlen(buf));
	nb_close(&s);
	return (r == 1 && n == -EPROTO && (s.last_ev & EPOLLERR) &&
		!strstr(buf, "SHOULD-NOT-ARRIVE")) ? 0 : -1;
}

/* N5 / N6: non-blocking connect that fails in the guard. */
static int case_nb_fail(const char *c, int port, int want_errno)
{
	struct nb s;
	int w, e1, e2, rc = -1;
	ssize_t wr;

	if (nb_connect(&s, c, port)) {
		nb_close(&s);
		return -1;
	}
	w = nb_wait_out_epoll(&s, 0);
	e1 = so_error(s.fd);
	e2 = so_error(s.fd);
	wr = write(s.fd, EARLY, sizeof(EARLY) - 1);
	printf("IGKM-E: %s wait result=%d events=%s SO_ERROR=%d (%s), SO_ERROR again=%d, write() afterwards=%ld errno=%d (%s) t=%s\n",
	       c, w, evs(s.last_ev), e1, e1 > 0 ? strerror(e1) : "-", e2,
	       (long)wr, wr < 0 ? errno : 0, wr < 0 ? strerror(errno) : "-",
	       mono());
	if (w == 1 && (s.last_ev & EPOLLERR) && !(s.last_ev & EPOLLOUT) &&
	    e1 == want_errno && e2 == 0 && wr < 0)
		rc = 0;
	nb_close(&s);
	return rc;
}

static int case_client_first(int *second)
{
	int err, rc, fd = open_to("E client-first", 6450, &err);

	*second = -1;
	if (fd < 0)
		return -1;
	rc = send_line("E client-first", fd,
		       "IGKM-E-REQ1 client-first guest->peer\n");
	if (!rc)
		rc = expect_line("E client-first", fd,
				 "IGKM-E-RESP1 client-first peer->guest\n");
	if (rc) {
		close(fd);
		return rc;
	}
	printf("IGKM-E: E exchange 1 done; sleeping %d s on the open connection (host kills the relay now) t=%s\n",
	       KILL_WAIT_S, mono());
	sleep(KILL_WAIT_S);
	printf("IGKM-E: E woke; exchange 2 on the SAME connection t=%s\n",
	       mono());
	*second = send_line("E client-first", fd,
			    "IGKM-E-REQ2 after-relay-kill guest->peer\n");
	if (!*second)
		*second = expect_line("E client-first", fd,
				      "IGKM-E-RESP2 after-relay-kill peer->guest\n");
	close(fd);
	return rc;
}

static int case_agent_down(void)
{
	int err, fd = open_to("F agent-down", 6443, &err);

	if (fd >= 0) {
		printf("IGKM-E: F agent-down: connect() SUCCEEDED without the relay\n");
		close(fd);
		return -1;
	}
	return err == ECONNABORTED ? 0 : -1;
}

/* SN (stretch): non-blocking listener under epoll. */
static int case_stretch(int *denied)
{
	struct sockaddr_in sa, peer;
	struct epoll_event ev;
	int lfd, ep, one = 1, conns = 0, ok = -1;

	*denied = -1;
	lfd = socket(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0);
	ep = epoll_create1(0);
	if (lfd < 0 || ep < 0)
		return -1;
	setsockopt(lfd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
	memset(&sa, 0, sizeof(sa));
	sa.sin_family = AF_INET;
	sa.sin_port = htons(7443);
	sa.sin_addr.s_addr = htonl(INADDR_ANY);
	if (bind(lfd, (struct sockaddr *)&sa, sizeof(sa)) || listen(lfd, 4))
		return -1;
	ev.events = EPOLLIN;
	ev.data.fd = lfd;
	epoll_ctl(ep, EPOLL_CTL_ADD, lfd, &ev);
	printf("IGKM-E: SN non-blocking listen on 0.0.0.0:7443 under epoll READY-FOR-INBOUND-NB t=%s\n",
	       mono());
	while (conns < 2 || ok < 0 || *denied < 0) {
		struct epoll_event out[4];
		int n = epoll_wait(ep, out, 4, 15000), i;

		if (n <= 0) {
			printf("IGKM-E: SN epoll_wait timed out\n");
			break;
		}
		for (i = 0; i < n; i++) {
			int fd = out[i].data.fd;

			if (fd == lfd) {
				socklen_t pl = sizeof(peer);
				unsigned long long t0 = now_ns();
				int c = accept4(lfd, (struct sockaddr *)&peer,
						&pl, SOCK_NONBLOCK);
				int e = c < 0 ? errno : 0;

				printf("IGKM-E: SN accept4(SOCK_NONBLOCK) on the non-blocking listener returned %d errno=%d after %llu us (no handshake hold) t=%s\n",
				       c, e, (now_ns() - t0) / 1000ull, mono());
				if (c < 0)
					continue;
				conns++;
				ev.events = EPOLLIN;
				ev.data.fd = c;
				epoll_ctl(ep, EPOLL_CTL_ADD, c, &ev);
				continue;
			}
			{
				char buf[256];
				ssize_t r = read(fd, buf, sizeof(buf) - 1);
				int e = r < 0 ? errno : 0;

				printf("IGKM-E: SN fd %d events=%s read()=%ld errno=%d (%s) t=%s\n",
				       fd, evs(out[i].events), (long)r, e,
				       e ? strerror(e) : "-", mono());
				if (r > 0) {
					buf[r] = '\0';
					printf("IGKM-E: SN fd %d read: %s", fd,
					       buf);
					if (!strcmp(buf, "IGKM-E-REQI inbound peer->guest\n") &&
					    !send_line("SN inbound", fd,
						       "IGKM-E-RESPI inbound guest->peer\n"))
						ok = 0;
					epoll_ctl(ep, EPOLL_CTL_DEL, fd, &ev);
					close(fd);
				} else if (r < 0 && e == EAGAIN) {
					continue;
				} else {
					if (r < 0 && e == ECONNABORTED)
						*denied = 0;
					epoll_ctl(ep, EPOLL_CTL_DEL, fd, &ev);
					close(fd);
				}
			}
		}
	}
	close(ep);
	close(lfd);
	return ok;
}

#define V(x) ((x) == 0 ? "PASS" : "FAIL")

int main(int argc, char *argv[])
{
	unsigned long long wall = (unsigned long long)ukplat_wall_clock();
	int i, fail = 0;

	printf("IGKM-E: BOOT-MARKER app started t=%s\n", mono());
	printf("IGKM-E: guest wall clock ukplat_wall_clock()=%llu.%06llu s since 1970-01-01\n",
	       wall / 1000000000ull, (wall % 1000000000ull) / 1000ull);
	for (i = 0; i < argc; i++)
		printf("IGKM-E: argv[%d]=\"%s\"\n", i, argv[i]);

	if (argc > 1 && !strcmp(argv[1], "stretch")) {
		int d, s = case_stretch(&d);

		printf("IGKM-E: VERDICT SN nonblocking-accept-allowed=%s\n", V(s));
		printf("IGKM-E: VERDICT SN nonblocking-accept-denied=%s\n", V(d));
		printf("IGKM-E: app returning t=%s\n", mono());
		return (s || d) ? 1 : 0;
	}

#define RUN(name, expr) do { int _r = (expr); \
	printf("IGKM-E: VERDICT %s=%s\n", name, V(_r)); fail |= !!_r; } while (0)
	{
		int s2 = -1, e2 = -1, s1;

		RUN("A pass-through", case_passthrough());
		RUN("B deny", case_deny());
		RUN("D server-first", case_server_first_blocking());
		s1 = case_inbound(&s2);
		RUN("S1 inbound-allowed", s1);
		RUN("S2 inbound-denied", s2);
		RUN("N1 nb-epoll-LT-client-first", case_n1());
		RUN("N2 nb-epoll-LT-server-first", case_n2());
		RUN("N3 nb-epoll-ET-1byte-bulk", case_n3());
		RUN("N4 nb-poll-1byte-server-first", case_n4());
		RUN("K keyupdate", case_keyupdate());
		RUN("R1 raw-nst-keyupdate-not-requested", case_raw_ok());
		RUN("R2 raw-unexpected-message", case_raw_bad());
		RUN("N5 nb-deny", case_nb_fail("N5 nb-deny", 6444, EACCES));
		RUN("E client-first", case_client_first(&e2));
		RUN("E after-relay-kill", e2);
		RUN("F agent-down", case_agent_down());
		RUN("N6 nb-agent-down", case_nb_fail("N6 nb-agent-down", 6443,
						     ECONNABORTED));
	}
	printf("IGKM-E: app returning t=%s\n", mono());
	return fail ? 1 : 0;
}
