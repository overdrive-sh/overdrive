/*
 * Spike B guest program (GH #303 in-guest-kernel-mtls, increment-d).
 *
 * An ORDINARY program using plain blocking TCP sockets. It contains no TLS,
 * no certificates and no knowledge of the mesh. Whether a connection is
 * encrypted is decided underneath it, by lib-mtlsguard and the host relay.
 *
 * Cases, in order (the host runner kills the relay during case E):
 *   A  pass-through : plain TCP to a non-mesh port (192.168.203.1:5001)
 *   B  deny         : mesh port whose peer identity the host policy denies
 *   C  nst-control  : mesh port whose peer sends a NewSessionTicket after
 *                     the handshake; the guest must fail closed on read
 *   D  server-first : read the peer's greeting first, then REQUEST/RESPONSE
 *   S  inbound      : a plain listen()/accept() server on :7443; the host runs
 *                     an mTLS client with an allowed identity (S1: accept()
 *                     returns, REQI/RESPI) and then a denied one (S2: accept()
 *                     fails with ECONNABORTED)
 *   E  client-first : write REQ1 immediately after connect() returns, read
 *                     RESP1; sleep while the host kills the relay; write
 *                     REQ2 / read RESP2 on the SAME connection
 *   F  agent-down   : a new mesh connect after the relay is dead
 *   G  non-blocking : a non-blocking mesh connect (out of scope -> refused)
 */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <uk/plat/time.h>

#define HOST_U32 0xC0A8CB01u /* 192.168.203.1 */
#define KILL_WAIT_S 8

static const char *mono(void)
{
	static char buf[32];
	unsigned long long ns = (unsigned long long)ukplat_monotonic_clock();

	snprintf(buf, sizeof(buf), "%llu.%06llu", ns / 1000000000ull,
		 (ns % 1000000000ull) / 1000ull);
	return buf;
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

static int open_to(const char *c, int port, int flags, int *err)
{
	struct sockaddr_in sa;
	unsigned long long t0;
	int fd, rc;

	fd = socket(AF_INET, SOCK_STREAM | flags, 0);
	if (fd < 0) {
		*err = errno;
		return -1;
	}
	memset(&sa, 0, sizeof(sa));
	sa.sin_family = AF_INET;
	sa.sin_port = htons(port);
	sa.sin_addr.s_addr = htonl(HOST_U32);
	printf("IGKM-D: %s connect(192.168.203.1:%d) t=%s\n", c, port, mono());
	t0 = (unsigned long long)ukplat_monotonic_clock();
	rc = connect(fd, (struct sockaddr *)&sa, sizeof(sa));
	*err = rc ? errno : 0;
	printf("IGKM-D: %s connect() returned %d errno=%d (%s) after %llu us t=%s\n",
	       c, rc, *err, *err ? strerror(*err) : "-",
	       ((unsigned long long)ukplat_monotonic_clock() - t0) / 1000ull,
	       mono());
	if (rc) {
		close(fd);
		return -1;
	}
	return fd;
}

static int send_line(const char *c, int fd, const char *line)
{
	if (write_all(fd, line, strlen(line)) < 0) {
		printf("IGKM-D: %s write FAILED errno=%d (%s)\n", c, errno,
		       strerror(errno));
		return -1;
	}
	printf("IGKM-D: %s wrote %lu bytes: %s", c,
	       (unsigned long)strlen(line), line);
	return 0;
}

static int expect_line(const char *c, int fd, const char *want)
{
	char buf[256];
	long n = read_line(fd, buf, sizeof(buf));

	if (n < 0) {
		printf("IGKM-D: %s read FAILED errno=%d (%s)\n", c, errno,
		       strerror(errno));
		return -errno;
	}
	printf("IGKM-D: %s read %ld bytes: %s%s", c, n, buf,
	       (n == 0 || buf[n - 1] != '\n') ? "\n" : "");
	if ((size_t)n != strlen(want) || memcmp(buf, want, (size_t)n)) {
		printf("IGKM-D: %s MISMATCH (want %s)", c, want);
		return -1;
	}
	return 0;
}

static int case_passthrough(void)
{
	int err, rc, fd = open_to("A pass-through", 5001, 0, &err);

	if (fd < 0)
		return -1;
	rc = send_line("A pass-through", fd,
		       "IGKM-D-PLAIN-REQ pass-through guest->host\n");
	if (!rc)
		rc = expect_line("A pass-through", fd,
				 "IGKM-D-PLAIN-RESP pass-through host->guest\n");
	close(fd);
	return rc;
}

static int case_deny(void)
{
	int err, fd = open_to("B deny", 6444, 0, &err);

	if (fd >= 0) {
		printf("IGKM-D: B deny: connect() SUCCEEDED (policy not enforced)\n");
		close(fd);
		return -1;
	}
	return err == EACCES ? 0 : -1;
}

static int case_nst(void)
{
	char buf[256];
	int err, fd = open_to("C nst-control", 6446, 0, &err);
	long n;

	if (fd < 0)
		return -1;
	if (send_line("C nst-control", fd, "IGKM-D-REQN nst guest->peer\n")) {
		close(fd);
		return -1;
	}
	n = read_line(fd, buf, sizeof(buf));
	err = n < 0 ? errno : 0;
	printf("IGKM-D: C nst-control read returned %ld errno=%d (%s)\n", n,
	       err, err ? strerror(err) : "-");
	close(fd);
	return (n < 0 && err == EPROTO) ? 0 : -1;
}

static int case_server_first(void)
{
	int err, rc, fd = open_to("D server-first", 6445, 0, &err);

	if (fd < 0)
		return -1;
	rc = expect_line("D server-first", fd,
			 "IGKM-D-GREETING server-first peer->guest\n");
	if (!rc)
		rc = send_line("D server-first", fd,
			       "IGKM-D-REQS server-first guest->peer\n");
	if (!rc)
		rc = expect_line("D server-first", fd,
				 "IGKM-D-RESPS server-first peer->guest\n");
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
		printf("IGKM-D: S bind/listen FAILED errno=%d\n", errno);
		close(lfd);
		return -1;
	}
	printf("IGKM-D: S listening on 0.0.0.0:7443 READY-FOR-INBOUND t=%s\n",
	       mono());
	fd = accept(lfd, (struct sockaddr *)&peer, &pl);
	printf("IGKM-D: S1 accept() returned %d errno=%d t=%s\n", fd,
	       fd < 0 ? errno : 0, mono());
	if (fd >= 0) {
		n = read_line(fd, buf, sizeof(buf));
		printf("IGKM-D: S1 read %ld bytes: %s%s", n, n > 0 ? buf : "",
		       (n <= 0 || buf[n - 1] != '\n') ? "\n" : "");
		if (n == (long)strlen("IGKM-D-REQI inbound peer->guest\n") &&
		    !memcmp(buf, "IGKM-D-REQI inbound peer->guest\n", (size_t)n))
			rc = send_line("S1 inbound", fd,
				       "IGKM-D-RESPI inbound guest->peer\n");
		close(fd);
	}
	pl = sizeof(peer);
	fd = accept(lfd, (struct sockaddr *)&peer, &pl);
	n = fd < 0 ? errno : 0;
	printf("IGKM-D: S2 accept() returned %d errno=%ld (%s) t=%s\n", fd, n,
	       n ? strerror((int)n) : "-", mono());
	if (fd >= 0)
		close(fd);
	else if (n == ECONNABORTED)
		*s2 = 0;
	close(lfd);
	return rc;
}

static int case_client_first(int *second)
{
	int err, rc, fd = open_to("E client-first", 6443, 0, &err);

	*second = -1;
	if (fd < 0)
		return -1;
	/* No delay: the very first write right after connect() returns. */
	rc = send_line("E client-first", fd,
		       "IGKM-D-REQ1 client-first guest->peer\n");
	if (!rc)
		rc = expect_line("E client-first", fd,
				 "IGKM-D-RESP1 client-first peer->guest\n");
	if (rc) {
		close(fd);
		return rc;
	}
	printf("IGKM-D: E exchange 1 done; sleeping %d s on the open connection (host kills the relay now) t=%s\n",
	       KILL_WAIT_S, mono());
	sleep(KILL_WAIT_S);
	printf("IGKM-D: E woke; exchange 2 on the SAME connection t=%s\n",
	       mono());
	*second = send_line("E client-first", fd,
			    "IGKM-D-REQ2 after-relay-kill guest->peer\n");
	if (!*second)
		*second = expect_line("E client-first", fd,
				      "IGKM-D-RESP2 after-relay-kill peer->guest\n");
	close(fd);
	return rc;
}

static int case_agent_down(void)
{
	int err, fd = open_to("F agent-down", 6443, 0, &err);

	if (fd >= 0) {
		printf("IGKM-D: F agent-down: connect() SUCCEEDED without the relay\n");
		close(fd);
		return -1;
	}
	return err == ECONNABORTED ? 0 : -1;
}

static int case_nonblocking(void)
{
	int err, fd = open_to("G non-blocking", 6443, SOCK_NONBLOCK, &err);

	if (fd >= 0) {
		close(fd);
		return -1;
	}
	return err == EOPNOTSUPP ? 0 : -1;
}

#define V(x) ((x) == 0 ? "PASS" : "FAIL")

int main(int argc, char *argv[])
{
	unsigned long long wall = (unsigned long long)ukplat_wall_clock();
	int a, b, c, d, s1, s2, e, e2, f, g, i;

	printf("IGKM-D: BOOT-MARKER app started t=%s\n", mono());
	printf("IGKM-D: guest wall clock ukplat_wall_clock()=%llu.%06llu s since 1970-01-01\n",
	       wall / 1000000000ull, (wall % 1000000000ull) / 1000ull);
	for (i = 0; i < argc; i++)
		printf("IGKM-D: argv[%d]=\"%s\"\n", i, argv[i]);

	a = case_passthrough();
	printf("IGKM-D: VERDICT A pass-through=%s\n", V(a));
	b = case_deny();
	printf("IGKM-D: VERDICT B deny=%s\n", V(b));
	c = case_nst();
	printf("IGKM-D: VERDICT C nst-control=%s\n", V(c));
	d = case_server_first();
	printf("IGKM-D: VERDICT D server-first=%s\n", V(d));
	s1 = case_inbound(&s2);
	printf("IGKM-D: VERDICT S1 inbound-allowed=%s\n", V(s1));
	printf("IGKM-D: VERDICT S2 inbound-denied=%s\n", V(s2));
	e = case_client_first(&e2);
	printf("IGKM-D: VERDICT E client-first=%s\n", V(e));
	printf("IGKM-D: VERDICT E after-relay-kill=%s\n", V(e2));
	f = case_agent_down();
	printf("IGKM-D: VERDICT F agent-down=%s\n", V(f));
	g = case_nonblocking();
	printf("IGKM-D: VERDICT G non-blocking=%s\n", V(g));
	printf("IGKM-D: app returning t=%s\n", mono());
	return (a || b || c || d || s1 || s2 || e || e2 || f || g) ? 1 : 0;
}
