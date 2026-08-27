/* PROBE increment-n — guest /init (PID 1) inside the Cloud-Hypervisor microVM.
 *
 * The guest's ONLY NIC is a virtio-net device backed by a tap inside the
 * per-workload netns. There is NO host `struct sock` for this connection — the
 * guest terminates TCP in ITS OWN kernel. The guest:
 *   1. brings up lo + eth0 (no kernel ip= autoconfig; CONFIG_IP_PNP unset), via
 *      raw ioctls (SIOCSIFADDR / SIOCSIFNETMASK / SIOCSIFFLAGS) + SIOCADDRT.
 *   2. connect()s to an arbitrary peer 10.99.0.1:9000 that is NOT present on the
 *      wire (proves INTERCEPTION, not routing to a real server).
 *   3. writes a byte-distinct REQUEST, reads the RESPONSE, prints BOTH + a
 *      SUCCESS/FAILURE verdict to the serial console, then powers off.
 *
 * Static build: gcc -static -O2 guest-init.c -o guest-init  (installed as /init)
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <fcntl.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <sys/select.h>
#include <sys/reboot.h>
#include <net/if.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <arpa/inet.h>
#include <net/route.h>

#define GUEST_IP   "10.77.0.2"
#define GUEST_MASK "255.255.255.252"
#define GUEST_GW   "10.77.0.1"
#define PEER_IP    "10.99.0.1"
#define PEER_PORT  9000
#define REQUEST    "PROBE-REQ-GUEST-TO-PEER-7\n"

static void say(const char *s) { write(1, s, strlen(s)); }

static int set_addr(int s, const char *ifn, unsigned long req, const char *ip) {
    struct ifreq ifr;
    memset(&ifr, 0, sizeof ifr);
    strncpy(ifr.ifr_name, ifn, IFNAMSIZ - 1);
    struct sockaddr_in *sin = (struct sockaddr_in *)&ifr.ifr_addr;
    sin->sin_family = AF_INET;
    inet_pton(AF_INET, ip, &sin->sin_addr);
    int rc = ioctl(s, req, &ifr);
    if (rc < 0) { char b[128]; snprintf(b, sizeof b, "  ioctl(%lu,%s,%s) FAILED errno=%d %s\n", req, ifn, ip, errno, strerror(errno)); say(b); }
    return rc;
}

static int if_up(int s, const char *ifn) {
    struct ifreq ifr;
    memset(&ifr, 0, sizeof ifr);
    strncpy(ifr.ifr_name, ifn, IFNAMSIZ - 1);
    if (ioctl(s, SIOCGIFFLAGS, &ifr) < 0) { say("  SIOCGIFFLAGS failed\n"); return -1; }
    ifr.ifr_flags |= IFF_UP | IFF_RUNNING;
    if (ioctl(s, SIOCSIFFLAGS, &ifr) < 0) { say("  SIOCSIFFLAGS failed\n"); return -1; }
    return 0;
}

/* Pick the first non-loopback interface (virtio-net -> typically eth0). */
static int find_nic(char *out) {
    struct if_nameindex *idx = if_nameindex(), *p;
    if (!idx) return -1;
    int found = -1;
    for (p = idx; p->if_index; p++) {
        char b[128]; snprintf(b, sizeof b, "  iface: %s (idx %u)\n", p->if_name, p->if_index); say(b);
        if (found < 0 && strcmp(p->if_name, "lo") != 0) {
            strncpy(out, p->if_name, IFNAMSIZ - 1);
            found = 0;
        }
    }
    if_freenameindex(idx);
    return found;
}

static int add_default_route(int s, const char *dev, const char *gw) {
    struct rtentry rt;
    memset(&rt, 0, sizeof rt);
    struct sockaddr_in *d = (struct sockaddr_in *)&rt.rt_dst;
    struct sockaddr_in *m = (struct sockaddr_in *)&rt.rt_genmask;
    struct sockaddr_in *g = (struct sockaddr_in *)&rt.rt_gateway;
    d->sin_family = m->sin_family = g->sin_family = AF_INET;
    d->sin_addr.s_addr = INADDR_ANY;   /* 0.0.0.0 */
    m->sin_addr.s_addr = INADDR_ANY;   /* 0.0.0.0 */
    inet_pton(AF_INET, gw, &g->sin_addr);
    char devbuf[IFNAMSIZ];
    strncpy(devbuf, dev, IFNAMSIZ - 1); devbuf[IFNAMSIZ-1] = 0;
    rt.rt_dev = devbuf;
    rt.rt_flags = RTF_UP | RTF_GATEWAY;
    int rc = ioctl(s, SIOCADDRT, &rt);
    if (rc < 0) { char b[128]; snprintf(b, sizeof b, "  SIOCADDRT default via %s failed errno=%d %s\n", gw, errno, strerror(errno)); say(b); }
    return rc;
}

/* Non-blocking connect with an 8s cap so a failed intercept reports FAILURE
 * instead of hanging until the run.sh watchdog. */
static int connect_timeout(int fd, struct sockaddr_in *peer, int secs) {
    int fl = fcntl(fd, F_GETFL, 0);
    fcntl(fd, F_SETFL, fl | O_NONBLOCK);
    int rc = connect(fd, (struct sockaddr *)peer, sizeof *peer);
    if (rc == 0) { fcntl(fd, F_SETFL, fl); return 0; }
    if (errno != EINPROGRESS) return -1;
    fd_set w; FD_ZERO(&w); FD_SET(fd, &w);
    struct timeval tv = { secs, 0 };
    rc = select(fd + 1, NULL, &w, NULL, &tv);
    if (rc <= 0) { errno = ETIMEDOUT; return -1; }
    int err = 0; socklen_t el = sizeof err;
    getsockopt(fd, SOL_SOCKET, SO_ERROR, &err, &el);
    fcntl(fd, F_SETFL, fl);
    if (err) { errno = err; return -1; }
    return 0;
}

static void finish(int code) {
    char b[64]; snprintf(b, sizeof b, "GUEST-EXITCODE=%d\n", code); say(b);
    sync();
    /* Clean poweroff so CH exits promptly; watchdog is the backstop. */
    reboot(RB_POWER_OFF);
    for (;;) pause();
}

int main(void) {
    say("\n==== GUEST INIT (pid 1) START ====\n");

    int s = socket(AF_INET, SOCK_DGRAM, 0);
    if (s < 0) { say("control socket failed\n"); finish(90); }

    if_up(s, "lo");

    char nic[IFNAMSIZ] = {0};
    if (find_nic(nic) < 0) { say("NO NON-LO NIC FOUND\n"); finish(91); }
    { char b[96]; snprintf(b, sizeof b, "selected NIC: %s\n", nic); say(b); }

    set_addr(s, nic, SIOCSIFADDR, GUEST_IP);
    set_addr(s, nic, SIOCSIFNETMASK, GUEST_MASK);
    if (if_up(s, nic) < 0) { say("NIC up failed\n"); finish(92); }
    add_default_route(s, nic, GUEST_GW);
    say("  eth0 configured " GUEST_IP "/30 gw " GUEST_GW "\n");

    /* Dial the arbitrary peer that is NOT on the wire — success proves the
     * host TRANSPARENTLY intercepted the egress. */
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in peer;
    memset(&peer, 0, sizeof peer);
    peer.sin_family = AF_INET;
    peer.sin_port = htons(PEER_PORT);
    inet_pton(AF_INET, PEER_IP, &peer.sin_addr);

    say("connect() -> " PEER_IP ":9000 ...\n");
    if (connect_timeout(fd, &peer, 8) < 0) {
        char b[128]; snprintf(b, sizeof b, "CONNECT FAILED errno=%d %s\n", errno, strerror(errno)); say(b);
        finish(10);
    }
    say("CONNECT OK\n");

    struct timeval rt = { 8, 0 };
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &rt, sizeof rt);

    if (write(fd, REQUEST, strlen(REQUEST)) < 0) { say("write REQUEST failed\n"); finish(11); }
    say("SENT REQUEST: " REQUEST);

    char buf[512];
    ssize_t n = read(fd, buf, sizeof buf - 1);
    if (n <= 0) {
        char b[128]; snprintf(b, sizeof b, "READ RESPONSE FAILED n=%zd errno=%d %s\n", n, errno, strerror(errno)); say(b);
        finish(12);
    }
    buf[n] = 0;
    { char b[600]; snprintf(b, sizeof b, "RECEIVED RESPONSE (%zd bytes): %s", n, buf); say(b); }

    /* Verdict: the response must be the host listener's byte-distinct reply. */
    if (strstr(buf, "PROBE-RESP-HOST-LISTENER-42")) {
        say(">>> ROUND-TRIP SUCCESS: guest received host's byte-distinct RESPONSE <<<\n");
        finish(0);
    }
    say(">>> ROUND-TRIP MISMATCH: unexpected response body <<<\n");
    finish(13);
    return 0;
}
