// V-10 guest side (THROWAWAY SPIKE). An ordinary AF_VSOCK client. It does NOT
// itself touch the packet header -- the loaded forge module (vsock_forge.ko)
// makes the guest kernel stamp a forged src_cid on everything this process
// sends. The client just dials the host (VMADDR_CID_HOST=2) on STREAM and
// SEQPACKET and reports connect()/send() results, so the serial log shows
// whether a forged-src_cid handshake completes.
//
// Modes:
//   plain                 : connect STREAM:sp + SEQPACKET:qp, send one marker each.
//   midstream <cid> <sp>  : STREAM connect with forge OFF, send MSG1-pre,
//                           write <cid> to the forge param, send MSG2-post,
//                           so the host sees MSG1 but the forged MSG2 is dropped
//                           while the connection stays attributed to the real CID.
//
// Usage:
//   guest_client plain <guest_cid> <stream_port> <seq_port>
//   guest_client midstream <guest_cid> <stream_port> <forge_cid>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <sys/socket.h>
#include <linux/vm_sockets.h>

#ifndef AF_VSOCK
#define AF_VSOCK 40
#endif

#define FORGE_PARAM "/sys/module/vsock_forge/parameters/forge_cid"

static int connect_to(int type, unsigned cid, unsigned port, int timeout_ms)
{
	int s = socket(AF_VSOCK, type, 0);
	if (s < 0) { printf("GUEST socket(type=%d) FAIL errno=%d(%s)\n", type, errno, strerror(errno)); return -1; }
	int fl = fcntl(s, F_GETFL, 0);
	fcntl(s, F_SETFL, fl | O_NONBLOCK);
	struct sockaddr_vm a;
	memset(&a, 0, sizeof a);
	a.svm_family = AF_VSOCK;
	a.svm_cid = cid;
	a.svm_port = port;
	int r = connect(s, (struct sockaddr *)&a, sizeof a);
	if (r < 0 && errno == EINPROGRESS) {
		struct pollfd p = { .fd = s, .events = POLLOUT };
		int pr = poll(&p, 1, timeout_ms);
		if (pr == 0) { printf("GUEST connect(type=%d port=%u) TIMEOUT after %dms (no host response)\n", type, port, timeout_ms); close(s); return -1; }
		int err = 0; socklen_t el = sizeof err;
		getsockopt(s, SOL_SOCKET, SO_ERROR, &err, &el);
		if (err) { printf("GUEST connect(type=%d port=%u) FAIL errno=%d(%s)\n", type, port, err, strerror(err)); close(s); return -1; }
	} else if (r < 0) {
		printf("GUEST connect(type=%d port=%u) FAIL errno=%d(%s)\n", type, port, errno, strerror(errno));
		close(s); return -1;
	}
	fcntl(s, F_SETFL, fl);
	printf("GUEST connect(type=%d port=%u) OK\n", type, port);
	return s;
}

static void write_forge_param(unsigned cid)
{
	FILE *f = fopen(FORGE_PARAM, "w");
	if (!f) { printf("GUEST set forge_cid=%u FAIL (%s: errno=%d)\n", cid, FORGE_PARAM, errno); return; }
	fprintf(f, "%u\n", cid);
	fclose(f);
	char rb[32] = {0};
	FILE *g = fopen(FORGE_PARAM, "r");
	if (g) { if (fgets(rb, sizeof rb, g)) { for (char *p = rb; *p; p++) if (*p=='\n') *p=0; } fclose(g); }
	printf("GUEST set forge_cid=%u (readback=%s)\n", cid, rb);
}

int main(int argc, char **argv)
{
	if (argc < 2) { fprintf(stderr, "usage: see header\n"); return 2; }
	const char *mode = argv[1];

	if (!strcmp(mode, "plain")) {
		unsigned gcid = argc > 2 ? strtoul(argv[2], 0, 10) : 0;
		unsigned sp = argc > 3 ? strtoul(argv[3], 0, 10) : 1234;
		unsigned qp = argc > 4 ? strtoul(argv[4], 0, 10) : 1235;
		printf("GUEST plain guest_cid=%u -> host(2) stream:%u seqpacket:%u\n", gcid, sp, qp);
		int s = connect_to(SOCK_STREAM, VMADDR_CID_HOST, sp, 6000);
		if (s >= 0) { char m[64]; int n = snprintf(m, sizeof m, "V10-STREAM-from-%u", gcid); send(s, m, n, 0); printf("GUEST stream sent \"%s\"\n", m); close(s); }
		int q = connect_to(SOCK_SEQPACKET, VMADDR_CID_HOST, qp, 6000);
		if (q >= 0) { char m[64]; int n = snprintf(m, sizeof m, "V10-SEQ-from-%u", gcid); send(q, m, n, 0); printf("GUEST seqpacket sent \"%s\"\n", m); close(q); }
		return 0;
	}

	if (!strcmp(mode, "midstream")) {
		unsigned gcid = argc > 2 ? strtoul(argv[2], 0, 10) : 0;
		unsigned sp = argc > 3 ? strtoul(argv[3], 0, 10) : 1234;
		unsigned forge = argc > 4 ? strtoul(argv[4], 0, 10) : 0;
		printf("GUEST midstream guest_cid=%u stream:%u forge->%u\n", gcid, sp, forge);
		int s = connect_to(SOCK_STREAM, VMADDR_CID_HOST, sp, 6000);
		if (s < 0) { printf("GUEST midstream connect FAILED (unexpected in control-first phase)\n"); return 1; }
		char m1[64]; int n1 = snprintf(m1, sizeof m1, "V10-MID-PRE-from-%u", gcid);
		send(s, m1, n1, 0); printf("GUEST midstream sent pre \"%s\"\n", m1);
		usleep(300000);
		write_forge_param(forge);
		usleep(300000);
		char m2[64]; int n2 = snprintf(m2, sizeof m2, "V10-MID-POST-forged-%u", forge);
		ssize_t w = send(s, m2, n2, 0);
		printf("GUEST midstream sent post rc=%zd errno=%d \"%s\"\n", w, errno, m2);
		usleep(500000);
		close(s);
		return 0;
	}

	fprintf(stderr, "unknown mode %s\n", mode);
	return 2;
}
