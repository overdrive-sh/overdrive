// V-10 host side (THROWAWAY SPIKE). Models ADR-0157's node-shared AF_VSOCK
// listener bound on VMADDR_CID_ANY, which attributes an incoming connection by
// its peer CID (getpeername -> svm_cid). We listen on BOTH a STREAM port and a
// SEQPACKET port (the design's two guest->host shapes, D4), accept, read the
// marker, and log the peer CID the HOST attributes to each accepted peer.
//
// The whole point: whatever the guest crafts into its packet `src_cid`, this
// program records the CID the host kernel actually delivered the connection as.
// If a forged src_cid were honoured, we would see the forged CID here.
//
// Usage: host_listener <stream_port> <seqpacket_port> <run_seconds> <logpath>

#include <stdio.h>
#include <stdlib.h>
#include <stdarg.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <time.h>
#include <poll.h>
#include <sys/socket.h>
#include <linux/vm_sockets.h>

#ifndef AF_VSOCK
#define AF_VSOCK 40
#endif

static FILE *LOG;

static void logln(const char *fmt, ...)
{
	char buf[512];
	va_list ap;
	struct timespec ts;
	clock_gettime(CLOCK_REALTIME, &ts);
	va_start(ap, fmt);
	vsnprintf(buf, sizeof buf, fmt, ap);
	va_end(ap);
	fprintf(LOG, "%ld.%03ld %s\n", (long)ts.tv_sec, ts.tv_nsec / 1000000, buf);
	fflush(LOG);
	printf("[host_listener] %s\n", buf);
	fflush(stdout);
}

static int mk_listener(int type, unsigned port)
{
	int s = socket(AF_VSOCK, type, 0);
	if (s < 0) { logln("socket(type=%d) errno=%d(%s)", type, errno, strerror(errno)); return -1; }
	struct sockaddr_vm a;
	memset(&a, 0, sizeof a);
	a.svm_family = AF_VSOCK;
	a.svm_cid = VMADDR_CID_ANY;
	a.svm_port = port;
	if (bind(s, (struct sockaddr *)&a, sizeof a) < 0) {
		logln("bind(port=%u type=%d) errno=%d(%s)", port, type, errno, strerror(errno));
		close(s); return -1;
	}
	if (listen(s, 64) < 0) {
		logln("listen(port=%u) errno=%d(%s)", port, errno, strerror(errno));
		close(s); return -1;
	}
	logln("listening type=%s port=%u on VMADDR_CID_ANY",
	      type == SOCK_STREAM ? "STREAM" : "SEQPACKET", port);
	return s;
}

static void handle(int ls, const char *kind)
{
	struct sockaddr_vm peer;
	socklen_t pl = sizeof peer;
	int c = accept(ls, (struct sockaddr *)&peer, &pl);
	if (c < 0) { logln("accept(%s) errno=%d(%s)", kind, errno, strerror(errno)); return; }
	struct sockaddr_vm pn;
	socklen_t nl = sizeof pn;
	unsigned peer_cid = peer.svm_cid, peer_port = peer.svm_port;
	if (getpeername(c, (struct sockaddr *)&pn, &nl) == 0)
		peer_cid = pn.svm_cid, peer_port = pn.svm_port;
	char buf[256];
	ssize_t n = recv(c, buf, sizeof buf - 1, 0);
	if (n < 0) n = 0;
	buf[n] = 0;
	for (ssize_t i = 0; i < n; i++) if (buf[i] == '\n') buf[i] = ' ';
	logln("ACCEPT %s peer_cid=%u peer_port=%u bytes=%zd data=\"%s\"",
	      kind, peer_cid, peer_port, n, buf);
	close(c);
}

int main(int argc, char **argv)
{
	if (argc < 5) { fprintf(stderr, "usage: %s stream_port seq_port seconds logpath\n", argv[0]); return 2; }
	unsigned sport = atoi(argv[1]), qport = atoi(argv[2]);
	int seconds = atoi(argv[3]);
	LOG = fopen(argv[4], "w");
	if (!LOG) { perror("fopen log"); return 2; }

	int sfd = mk_listener(SOCK_STREAM, sport);
	int qfd = mk_listener(SOCK_SEQPACKET, qport);
	logln("ready stream_fd=%d seqpacket_fd=%d run=%ds", sfd, qfd, seconds);

	time_t end = time(NULL) + seconds;
	int accepts = 0;
	while (time(NULL) < end) {
		struct pollfd pfd[2];
		int nf = 0;
		if (sfd >= 0) { pfd[nf].fd = sfd; pfd[nf].events = POLLIN; nf++; }
		if (qfd >= 0) { pfd[nf].fd = qfd; pfd[nf].events = POLLIN; nf++; }
		if (nf == 0) break;
		int r = poll(pfd, nf, 500);
		if (r <= 0) continue;
		for (int i = 0; i < nf; i++) {
			if (!(pfd[i].revents & POLLIN)) continue;
			if (pfd[i].fd == sfd) { handle(sfd, "STREAM"); accepts++; }
			else if (pfd[i].fd == qfd) { handle(qfd, "SEQPACKET"); accepts++; }
		}
	}
	logln("SUMMARY accepts=%d", accepts);
	if (sfd >= 0) close(sfd);
	if (qfd >= 0) close(qfd);
	fclose(LOG);
	return 0;
}
