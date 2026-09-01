/* PROBE increment-n — host-side IP_TRANSPARENT listener (leg-F analogue).
 *
 * Mirrors the production leg-F listener: bind an IP_TRANSPARENT socket on
 * 127.0.0.1:<PORT>, accept a TPROXY-diverted connection, recover the ORIGINAL
 * destination via getsockname() (TPROXY preserves it), read the guest's
 * REQUEST, write a BYTE-DISTINCT RESPONSE (not an echo — proves the real
 * server->client pipe), and log every fact.
 *
 * Static build: gcc -static -O2 listener.c -o host-listener
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <time.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>

#ifndef IP_TRANSPARENT
#define IP_TRANSPARENT 19
#endif

/* Byte-distinct REQUEST/RESPONSE litmus — the response is NOT the request, so
 * a passing round-trip proves the genuine server->client reply pipe. */
#define RESPONSE "PROBE-RESP-HOST-LISTENER-42\n"

static void ts(void) {
    struct timespec t;
    clock_gettime(CLOCK_REALTIME, &t);
    printf("[listener %ld.%03ld] ", (long)t.tv_sec, t.tv_nsec / 1000000);
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    if (argc < 2) { fprintf(stderr, "usage: %s <port>\n", argv[0]); return 2; }
    int port = atoi(argv[1]);

    int lfd = socket(AF_INET, SOCK_STREAM, 0);
    if (lfd < 0) { perror("socket"); return 1; }

    int one = 1;
    if (setsockopt(lfd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one) < 0)
        perror("SO_REUSEADDR");

    /* THE load-bearing option: IP_TRANSPARENT lets us bind a socket that
     * receives TPROXY-redirected connections and reply from the foreign
     * (original) destination address. */
    if (setsockopt(lfd, IPPROTO_IP, IP_TRANSPARENT, &one, sizeof one) < 0) {
        perror("IP_TRANSPARENT");
        return 1;
    }
    ts(); printf("IP_TRANSPARENT set OK\n");

    struct sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_addr.s_addr = inet_addr("127.0.0.1");
    a.sin_port = htons(port);
    if (bind(lfd, (struct sockaddr *)&a, sizeof a) < 0) { perror("bind"); return 1; }
    if (listen(lfd, 8) < 0) { perror("listen"); return 1; }
    ts(); printf("listening on 127.0.0.1:%d\n", port);

    /* Handle a couple of connections then exit (the probe drives exactly one). */
    for (int i = 0; i < 2; i++) {
        struct sockaddr_in peer;
        socklen_t plen = sizeof peer;
        int cfd = accept(lfd, (struct sockaddr *)&peer, &plen);
        if (cfd < 0) { perror("accept"); return 1; }

        /* getsockname on the accepted socket = the ORIGINAL destination the
         * guest dialed (TPROXY preserved it). MUST be 10.99.0.1:9000, NOT
         * 127.0.0.1:<PORT>. */
        struct sockaddr_in loc;
        socklen_t llen = sizeof loc;
        getsockname(cfd, (struct sockaddr *)&loc, &llen);

        char pbuf[64], lbuf[64];
        inet_ntop(AF_INET, &peer.sin_addr, pbuf, sizeof pbuf);
        inet_ntop(AF_INET, &loc.sin_addr, lbuf, sizeof lbuf);
        ts(); printf("ACCEPT peer=%s:%d  ORIG-DST(getsockname)=%s:%d\n",
                     pbuf, ntohs(peer.sin_port), lbuf, ntohs(loc.sin_port));

        char rbuf[512];
        ssize_t n = read(cfd, rbuf, sizeof rbuf - 1);
        if (n > 0) {
            rbuf[n] = 0;
            /* strip trailing newline for the log line */
            ts(); printf("READ REQUEST (%zd bytes): %.*s\n", n,
                         (int)(n && rbuf[n-1]=='\n' ? n-1 : n), rbuf);
        } else {
            ts(); printf("READ returned %zd (errno=%d %s)\n", n, errno, strerror(errno));
        }

        ssize_t w = write(cfd, RESPONSE, strlen(RESPONSE));
        ts(); printf("WROTE RESPONSE (%zd bytes): %s", w, RESPONSE);
        close(cfd);
        /* One successful exchange is the whole verdict. */
        ts(); printf("exchange %d complete\n", i);
        break;
    }
    return 0;
}
