/*
 * Spike A guest program (GH #303 in-guest-kernel-mtls). Runs as the Nanos
 * `program`. Static Linux ELF.
 *
 *   1. prints a boot marker (+ guest CLOCK_MONOTONIC)
 *   2. AF_VSOCK connect to CID 2 (host) port VSOCK_PORT, writes a distinct
 *      REQUEST, reads a distinct RESPONSE, compares it
 *   3. AF_INET connect over virtio-net to the host tap address, same shape
 *   4. prints a verdict line per leg and exits (0 only if both legs pass)
 *
 * Byte-distinct request/response so the check proves a real host->guest
 * reply pipe, not an echo.
 */
#include <arpa/inet.h>
#include <errno.h>
#include <linux/vm_sockets.h>
#include <netinet/in.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

#define VSOCK_PORT 5000
#define NET_HOST "192.168.203.1"
#define NET_PORT 5001

static const char VSOCK_REQ[] = "IGKM-A-VSOCK-REQUEST guest->host\n";
static const char VSOCK_RESP[] = "IGKM-A-VSOCK-RESPONSE host->guest\n";
static const char NET_REQ[] = "IGKM-A-NET-REQUEST guest->host\n";
static const char NET_RESP[] = "IGKM-A-NET-RESPONSE host->guest\n";

static double mono(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec / 1e9;
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

/* Read until '\n' or EOF; returns bytes read, -1 on error. */
static ssize_t read_line(int fd, char *buf, size_t cap)
{
    size_t got = 0;
    while (got + 1 < cap) {
        ssize_t n = read(fd, buf + got, 1);
        if (n < 0) {
            if (errno == EINTR)
                continue;
            return -1;
        }
        if (n == 0)
            break;
        got += (size_t)n;
        if (buf[got - 1] == '\n')
            break;
    }
    buf[got] = '\0';
    return (ssize_t)got;
}

static int exchange(const char *leg, int fd, const char *req, const char *expect)
{
    char buf[256];
    if (write_all(fd, req, strlen(req)) < 0) {
        printf("IGKM-A: %s write FAILED errno=%d (%s)\n", leg, errno, strerror(errno));
        return -1;
    }
    printf("IGKM-A: %s wrote %zu bytes t=%.6f\n", leg, strlen(req), mono());
    ssize_t n = read_line(fd, buf, sizeof(buf));
    if (n < 0) {
        printf("IGKM-A: %s read FAILED errno=%d (%s)\n", leg, errno, strerror(errno));
        return -1;
    }
    printf("IGKM-A: %s read %zd bytes: %.*s", leg, n, (int)n, buf);
    if (n == 0 || buf[n - 1] != '\n')
        printf("\n");
    if ((size_t)n != strlen(expect) || memcmp(buf, expect, (size_t)n) != 0) {
        printf("IGKM-A: %s RESPONSE MISMATCH\n", leg);
        return -1;
    }
    return 0;
}

static int vsock_leg(void)
{
    printf("IGKM-A: vsock socket(AF_VSOCK=%d, SOCK_STREAM) t=%.6f\n", AF_VSOCK, mono());
    int fd = socket(AF_VSOCK, SOCK_STREAM, 0);
    if (fd < 0) {
        printf("IGKM-A: vsock socket FAILED errno=%d (%s)\n", errno, strerror(errno));
        return -1;
    }
    struct sockaddr_vm sa;
    memset(&sa, 0, sizeof(sa));
    sa.svm_family = AF_VSOCK;
    sa.svm_cid = VMADDR_CID_HOST; /* 2 */
    sa.svm_port = VSOCK_PORT;
    if (connect(fd, (struct sockaddr *)&sa, sizeof(sa)) < 0) {
        printf("IGKM-A: vsock connect(cid=2, port=%d) FAILED errno=%d (%s)\n", VSOCK_PORT,
               errno, strerror(errno));
        close(fd);
        return -1;
    }
    struct sockaddr_vm local;
    socklen_t ll = sizeof(local);
    memset(&local, 0, sizeof(local));
    if (getsockname(fd, (struct sockaddr *)&local, &ll) == 0)
        printf("IGKM-A: vsock connected local cid=%u port=%u -> cid=2 port=%d t=%.6f\n",
               local.svm_cid, local.svm_port, VSOCK_PORT, mono());
    else
        printf("IGKM-A: vsock connected (getsockname errno=%d) t=%.6f\n", errno, mono());
    int rc = exchange("vsock", fd, VSOCK_REQ, VSOCK_RESP);
    close(fd);
    return rc;
}

static int net_leg(void)
{
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) {
        printf("IGKM-A: net socket FAILED errno=%d (%s)\n", errno, strerror(errno));
        return -1;
    }
    struct sockaddr_in sa;
    memset(&sa, 0, sizeof(sa));
    sa.sin_family = AF_INET;
    sa.sin_port = htons(NET_PORT);
    inet_pton(AF_INET, NET_HOST, &sa.sin_addr);
    if (connect(fd, (struct sockaddr *)&sa, sizeof(sa)) < 0) {
        printf("IGKM-A: net connect(%s:%d) FAILED errno=%d (%s)\n", NET_HOST, NET_PORT, errno,
               strerror(errno));
        close(fd);
        return -1;
    }
    struct sockaddr_in local;
    socklen_t ll = sizeof(local);
    char lbuf[INET_ADDRSTRLEN] = "?";
    if (getsockname(fd, (struct sockaddr *)&local, &ll) == 0)
        inet_ntop(AF_INET, &local.sin_addr, lbuf, sizeof(lbuf));
    printf("IGKM-A: net connected local %s:%u -> %s:%d t=%.6f\n", lbuf, ntohs(local.sin_port),
           NET_HOST, NET_PORT, mono());
    int rc = exchange("net", fd, NET_REQ, NET_RESP);
    close(fd);
    return rc;
}

int main(void)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    printf("IGKM-A: BOOT-MARKER guest program started t=%.6f\n", mono());
    int v = vsock_leg();
    printf("IGKM-A: VERDICT vsock=%s\n", v == 0 ? "PASS" : "FAIL");
    int n = net_leg();
    printf("IGKM-A: VERDICT net=%s\n", n == 0 ? "PASS" : "FAIL");
    printf("IGKM-A: program exiting t=%.6f\n", mono());
    return (v == 0 && n == 0) ? 0 : 1;
}
