/*
 * Spike A3 guest program (GH #303 in-guest-kernel-mtls): a native Unikraft
 * application, built for the KVM platform with the Firecracker VMM target.
 *
 *   1. prints a boot marker (+ Unikraft monotonic clock) and its argv
 *   2. AF_VSOCK connect to CID 2 (host) port VSOCK_PORT, writes a distinct
 *      REQUEST, reads a distinct RESPONSE, compares it
 *   3. AF_INET (lwIP) connect over virtio-net to the host tap address, same shape
 *   4. prints a verdict line per leg and returns (0 only if both legs pass)
 *
 * The REQUEST/RESPONSE bytes are the Nanos probes' (increment-a/-b), so the
 * host listener is reused unchanged. Byte-distinct request/response so the
 * check proves a real host->guest reply pipe, not an echo.
 */
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <linux/vm_sockets.h>
#include <uk/plat/time.h>

#ifndef AF_VSOCK
#define AF_VSOCK 40
#endif

#define VSOCK_PORT 5000
#define NET_PORT 5001
/* 192.168.203.1, the host tap address, in host byte order. */
#define NET_HOST_U32 0xC0A8CB01u

static const char VSOCK_REQ[] = "IGKM-A-VSOCK-REQUEST guest->host\n";
static const char VSOCK_RESP[] = "IGKM-A-VSOCK-RESPONSE host->guest\n";
static const char NET_REQ[] = "IGKM-A-NET-REQUEST guest->host\n";
static const char NET_RESP[] = "IGKM-A-NET-RESPONSE host->guest\n";

/* Unikraft monotonic clock (ns since boot) as "s.uuuuuu". */
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

/* Read until '\n' or EOF (chunked reads); returns bytes read, -1 on error. */
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

static int exchange(const char *leg, int fd, const char *req, const char *expect)
{
	char buf[256];
	long n;

	if (write_all(fd, req, strlen(req)) < 0) {
		printf("IGKM-C: %s write FAILED errno=%d (%s)\n", leg, errno,
		       strerror(errno));
		return -1;
	}
	printf("IGKM-C: %s wrote %lu bytes t=%s\n", leg,
	       (unsigned long)strlen(req), mono());
	n = read_line(fd, buf, sizeof(buf));
	if (n < 0) {
		printf("IGKM-C: %s read FAILED errno=%d (%s)\n", leg, errno,
		       strerror(errno));
		return -1;
	}
	printf("IGKM-C: %s read %ld bytes: %s", leg, n, buf);
	if (n == 0 || buf[n - 1] != '\n')
		printf("\n");
	if ((size_t)n != strlen(expect) || memcmp(buf, expect, (size_t)n) != 0) {
		printf("IGKM-C: %s RESPONSE MISMATCH\n", leg);
		return -1;
	}
	return 0;
}

static int vsock_leg(void)
{
	struct sockaddr_vm sa, local;
	socklen_t ll = sizeof(local);
	int fd, rc;

	printf("IGKM-C: vsock socket(AF_VSOCK=%d, SOCK_STREAM) t=%s\n",
	       AF_VSOCK, mono());
	fd = socket(AF_VSOCK, SOCK_STREAM, 0);
	if (fd < 0) {
		printf("IGKM-C: vsock socket FAILED errno=%d (%s)\n", errno,
		       strerror(errno));
		return -1;
	}
	memset(&sa, 0, sizeof(sa));
	sa.svm_family = AF_VSOCK;
	sa.svm_cid = VMADDR_CID_HOST; /* 2 */
	sa.svm_port = VSOCK_PORT;
	if (connect(fd, (struct sockaddr *)&sa, sizeof(sa)) < 0) {
		printf("IGKM-C: vsock connect(cid=2, port=%d) FAILED errno=%d (%s)\n",
		       VSOCK_PORT, errno, strerror(errno));
		close(fd);
		return -1;
	}
	memset(&local, 0, sizeof(local));
	if (getsockname(fd, (struct sockaddr *)&local, &ll) == 0)
		printf("IGKM-C: vsock connected local cid=%u port=%u -> cid=2 port=%d t=%s\n",
		       (unsigned int)local.svm_cid,
		       (unsigned int)local.svm_port, VSOCK_PORT, mono());
	else
		printf("IGKM-C: vsock connected (getsockname errno=%d) t=%s\n",
		       errno, mono());
	rc = exchange("vsock", fd, VSOCK_REQ, VSOCK_RESP);
	close(fd);
	return rc;
}

static int net_leg(void)
{
	struct sockaddr_in sa, local;
	socklen_t ll = sizeof(local);
	unsigned int a;
	int fd, rc;

	printf("IGKM-C: net socket(AF_INET, SOCK_STREAM) t=%s\n", mono());
	fd = socket(AF_INET, SOCK_STREAM, 0);
	if (fd < 0) {
		printf("IGKM-C: net socket FAILED errno=%d (%s)\n", errno,
		       strerror(errno));
		return -1;
	}
	memset(&sa, 0, sizeof(sa));
	sa.sin_family = AF_INET;
	sa.sin_port = htons(NET_PORT);
	sa.sin_addr.s_addr = htonl(NET_HOST_U32);
	if (connect(fd, (struct sockaddr *)&sa, sizeof(sa)) < 0) {
		printf("IGKM-C: net connect(192.168.203.1:%d) FAILED errno=%d (%s)\n",
		       NET_PORT, errno, strerror(errno));
		close(fd);
		return -1;
	}
	memset(&local, 0, sizeof(local));
	if (getsockname(fd, (struct sockaddr *)&local, &ll) == 0) {
		a = ntohl(local.sin_addr.s_addr);
		printf("IGKM-C: net connected local %u.%u.%u.%u:%u -> 192.168.203.1:%d t=%s\n",
		       (a >> 24) & 0xff, (a >> 16) & 0xff, (a >> 8) & 0xff,
		       a & 0xff, (unsigned int)ntohs(local.sin_port),
		       NET_PORT, mono());
	}
	rc = exchange("net", fd, NET_REQ, NET_RESP);
	close(fd);
	return rc;
}

int main(int argc, char *argv[])
{
	unsigned long long wall;
	int vs, nt, i;

	printf("IGKM-C: BOOT-MARKER native Unikraft app started t=%s\n", mono());
	/* Diagnostic (added after run 0007): Unikraft's wall clock is seeded from
	 * the CMOS RTC, which Firecracker v1.17.0 does not emulate on x86. */
	wall = (unsigned long long)ukplat_wall_clock();
	printf("IGKM-C: wall clock ukplat_wall_clock()=%llu.%06llu s since 1970-01-01\n",
	       wall / 1000000000ull, (wall % 1000000000ull) / 1000ull);
	for (i = 0; i < argc; i++)
		printf("IGKM-C: argv[%d]=\"%s\"\n", i, argv[i]);

	vs = vsock_leg();
	printf("IGKM-C: VERDICT vsock=%s\n", vs == 0 ? "PASS" : "FAIL");
	nt = net_leg();
	printf("IGKM-C: VERDICT net=%s\n", nt == 0 ? "PASS" : "FAIL");

	printf("IGKM-C: app returning from main t=%s\n", mono());
	return (vs == 0 && nt == 0) ? 0 : 1;
}
