/* SPDX-License-Identifier: BSD-3-Clause */
/*
 * Spike B (Overdrive GH #303, increment-d): in-guest transparent mTLS for a
 * Unikraft unikernel, with the private key and the TLS handshake on the host.
 * THROWAWAY PROBE CODE.
 *
 * Mechanism, for a blocking AF_INET/SOCK_STREAM connect() to a configured
 * mesh destination:
 *
 *  1. HOOK  - the patched core connect() calls uk_socket_connect_hook() once
 *             the lwIP connect has completed (TCP is up).
 *  2. HOLD  - the hook does not return until the TLS handshake is done, so
 *             the application cannot read or write before the record layer
 *             is installed. It also holds the socket's write lock, so a second
 *             thread's read()/write() on the fd blocks too.
 *  3. RELAY - the hook opens a vsock connection to the host (CID 2,
 *             mtlsguard.agent_port) and follows the host's lock-step frames:
 *             TO_PEER (write TLS bytes to the TCP peer), NEED_PEER (read once
 *             from the TCP peer, reply FROM_PEER), then SECRETS, DENY or ERROR.
 *             The host runs the rustls client and holds the certificate key.
 *  4. KEYS  - SECRETS carries only per-direction AES-128-GCM key, IV and
 *             sequence number. The vsock connection is closed afterwards.
 *  5. RECORD LAYER - the socket's per-instance driver pointer is swapped to a
 *             per-socket "guard" driver whose read/write/recv/send ops wrap
 *             the lwIP ops in a TLS 1.3 record layer (Mbed TLS AES-GCM, nonce =
 *             IV XOR seq, 5-byte header as AAD, inner type application_data).
 *             Any non-application record after the handshake fails closed
 *             (close_notify maps to EOF).
 *  6. POLICY - on DENY/ERROR/agent-unreachable the TCP connection is shut down
 *             and connect() fails; no application byte ever leaves the guest.
 *
 * The same mechanism runs on accept() (patch 0002): for a connection accepted
 * on a mesh listener (mtlsguard.mesh_in) the accept post-hook holds the new
 * socket before it gets an fd, the host runs the rustls SERVER with the
 * workload's server SVID and decides policy on the client's SPIFFE ID, and
 * the same record layer is installed.
 *
 * The guest relays handshake bytes ONE TLS RECORD per NEED_PEER, so it never
 * reads past the record that completes the handshake (the client's first
 * application record can follow its Finished in the same TCP segment).
 *
 * Out of scope (spike): non-blocking sockets and poll/epoll readiness (a
 * non-blocking mesh connect/accept is refused with EOPNOTSUPP), KeyUpdate,
 * session resumption, MSG_PEEK, TCP Fast Open.
 */
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/uio.h>
#include <netinet/in.h>
#include <linux/vm_sockets.h>

#include <uk/alloc.h>
#include <uk/assert.h>
#include <uk/essentials.h>
#include <uk/errptr.h>
#include <uk/file.h>
#include <uk/libparam.h>
#include <uk/plat/time.h>
#include <uk/posix-fd.h>
#include <uk/socket.h>
#include <uk/socket_driver.h>

#include <mbedtls/gcm.h>
#include <mbedtls/platform_util.h>
#include "aesni.h" /* Mbed TLS internal: mbedtls_aesni_has_support() */

#ifndef AF_VSOCK
#define AF_VSOCK 40
#endif

/* ---- configuration (library parameters) --------------------------------- */

/* mtlsguard.mesh=A.B.C.D:LO-HI[,A.B.C.D:P...]  (empty: hook is a no-op) */
static char *mesh;
UK_LIBPARAM_PARAM(mesh, charp, "mesh destinations ip:port[-port],...");
/* mtlsguard.mesh_in=A.B.C.D:LO-HI[,...]  local addresses of mesh listeners */
static char *mesh_in;
UK_LIBPARAM_PARAM(mesh_in, charp, "mesh listeners ip:port[-port],...");
/* mtlsguard.agent_port=<vsock port of the host relay on CID 2> */
static __u32 agent_port = 7100;
UK_LIBPARAM_PARAM(agent_port, __u32, "host relay vsock port");

#define HANDSHAKE_TIMEOUT_NS (10ULL * 1000000000ULL)
#define IO_TIMEOUT_NS        (30ULL * 1000000000ULL)

/* ---- relay protocol ----------------------------------------------------- */

#define T_OPEN      0x01
#define T_TO_PEER   0x02
#define T_NEED_PEER 0x03
#define T_FROM_PEER 0x04
#define T_SECRETS   0x05
#define T_DENY      0x06
#define T_ERROR     0x07
#define T_ACCEPT    0x08
#define MAX_FRAME   (64 * 1024)

static const char *tname(unsigned int t)
{
	switch (t) {
	case T_OPEN: return "OPEN";
	case T_TO_PEER: return "TO_PEER";
	case T_NEED_PEER: return "NEED_PEER";
	case T_FROM_PEER: return "FROM_PEER";
	case T_SECRETS: return "SECRETS";
	case T_DENY: return "DENY";
	case T_ERROR: return "ERROR";
	case T_ACCEPT: return "ACCEPT";
	default: return "UNKNOWN";
	}
}

/* ---- TLS 1.3 record layer ----------------------------------------------- */

#define REC_HDR       5
#define REC_TAG       16
#define REC_MAX_PLAIN 16384
#define REC_MAX_CT    (REC_MAX_PLAIN + 256)
#define CT_APPDATA    0x17
#define CT_ALERT      0x15
#define CT_HANDSHAKE  0x16

struct guard_sock {
	struct posix_socket_driver drv;     /* per-socket driver: .private = this */
	struct posix_socket_driver *orig;   /* lwIP's AF_INET driver */
	unsigned int id;
	mbedtls_gcm_context tx, rx;
	__u8 tx_iv[12], rx_iv[12];
	__u64 tx_seq, rx_seq;
	int rx_eof;
	int err;                             /* sticky, fail-closed */
	size_t rlen;                         /* ciphertext bytes in rbuf */
	size_t poff, plen;                   /* pending plaintext in pbuf */
	__u8 rbuf[REC_HDR + REC_MAX_CT];
	__u8 pbuf[REC_MAX_CT];
	__u8 sbuf[REC_HDR + REC_MAX_PLAIN + 1 + REC_TAG];
	char peer[128];
};

static unsigned int conn_seq;

static const char *mono(void)
{
	static char buf[32];
	unsigned long long ns = (unsigned long long)ukplat_monotonic_clock();

	snprintf(buf, sizeof(buf), "%llu.%06llu", ns / 1000000000ull,
		 (ns % 1000000000ull) / 1000ull);
	return buf;
}

static void fmt_dst(char *out, size_t n, const struct sockaddr_in *sin)
{
	__u32 a = ntohl(sin->sin_addr.s_addr);

	snprintf(out, n, "%u.%u.%u.%u:%u", (a >> 24) & 0xff, (a >> 16) & 0xff,
		 (a >> 8) & 0xff, a & 0xff, (unsigned int)ntohs(sin->sin_port));
}

/* ---- mesh destination matching ------------------------------------------ */

static int parse_u(const char **p, unsigned long *v)
{
	const char *s = *p;
	unsigned long x = 0;

	if (*s < '0' || *s > '9')
		return -1;
	while (*s >= '0' && *s <= '9')
		x = x * 10 + (unsigned long)(*s++ - '0');
	*p = s;
	*v = x;
	return 0;
}

static int is_mesh(const char *list, const struct sockaddr_in *sin)
{
	__u32 ip = ntohl(sin->sin_addr.s_addr);
	unsigned int port = ntohs(sin->sin_port);
	const char *p = list;

	while (p && *p) {
		unsigned long o[4], lo, hi;
		int i;

		for (i = 0; i < 4; i++) {
			if (parse_u(&p, &o[i]) || o[i] > 255)
				return 0;
			if (i < 3 && *p++ != '.')
				return 0;
		}
		if (*p++ != ':' || parse_u(&p, &lo))
			return 0;
		hi = lo;
		if (*p == '-') {
			p++;
			if (parse_u(&p, &hi))
				return 0;
		}
		if (ip == ((o[0] << 24) | (o[1] << 16) | (o[2] << 8) | o[3]) &&
		    port >= lo && port <= hi)
			return 1;
		if (*p == ',')
			p++;
	}
	return 0;
}

/* ---- blocking helpers on a posix_sock with a deadline -------------------- */

static ssize_t xread(const struct uk_file *f, void *buf, size_t n,
		     __nsec deadline)
{
	struct iovec iov = { .iov_base = buf, .iov_len = n };
	ssize_t r;

	for (;;) {
		r = posix_socket_read(f, &iov, 1);
		if (r != -EAGAIN)
			return r;
		if (!uk_file_poll_until(f, UKFD_POLLIN | UKFD_POLL_ALWAYS,
					deadline))
			return -ETIMEDOUT;
	}
}

static int xread_exact(const struct uk_file *f, void *buf, size_t n,
		       __nsec deadline)
{
	size_t got = 0;
	ssize_t r;

	while (got < n) {
		r = xread(f, (__u8 *)buf + got, n - got, deadline);
		if (r < 0)
			return (int)r;
		if (r == 0)
			return -ECONNRESET;
		got += (size_t)r;
	}
	return 0;
}

static int xwrite_all(const struct uk_file *f, const void *buf, size_t n,
		      __nsec deadline)
{
	size_t off = 0;
	ssize_t r;

	while (off < n) {
		struct iovec iov = { .iov_base = (__u8 *)buf + off,
				     .iov_len = n - off };

		r = posix_socket_write(f, &iov, 1);
		if (r > 0) {
			off += (size_t)r;
			continue;
		}
		if (r == 0)
			return -EPIPE;
		if (r != -EAGAIN)
			return (int)r;
		if (!uk_file_poll_until(f, UKFD_POLLOUT | UKFD_POLL_ALWAYS,
					deadline))
			return -ETIMEDOUT;
	}
	return 0;
}

/* Same as above, but through an explicit (original lwIP) driver. */
static int orig_write_all(struct guard_sock *gs, const struct uk_file *f,
			  const void *buf, size_t n, __nsec deadline)
{
	size_t off = 0;
	ssize_t r;

	while (off < n) {
		struct iovec iov = { .iov_base = (__u8 *)buf + off,
				     .iov_len = n - off };

		r = gs->orig->ops->write(f, &iov, 1);
		if (r > 0) {
			off += (size_t)r;
			continue;
		}
		if (r == 0)
			return -EPIPE;
		if (r != -EAGAIN)
			return (int)r;
		if (!uk_file_poll_until(f, UKFD_POLLOUT | UKFD_POLL_ALWAYS,
					deadline))
			return -ETIMEDOUT;
	}
	return 0;
}

/* ---- frames -------------------------------------------------------------- */

static int frame_send(const struct uk_file *vs, unsigned int id, __u8 t,
		      const void *p, __u32 len, __nsec deadline)
{
	__u8 hdr[5] = { t, (__u8)(len >> 24), (__u8)(len >> 16),
			(__u8)(len >> 8), (__u8)len };
	int r = xwrite_all(vs, hdr, sizeof(hdr), deadline);

	if (!r && len)
		r = xwrite_all(vs, p, len, deadline);
	printf("MTLSGUARD: [c%u] vsock tx %s payload=%u wire=%u%s t=%s\n", id,
	       tname(t), (unsigned int)len, (unsigned int)len + 5,
	       r ? " FAILED" : "", mono());
	return r;
}

/* Receives one frame into buf (cap bytes); returns payload length or <0. */
static long frame_recv(const struct uk_file *vs, __u8 *t, __u8 *buf,
		       size_t cap, __nsec deadline)
{
	__u8 hdr[5];
	__u32 len;
	int r = xread_exact(vs, hdr, sizeof(hdr), deadline);

	if (r)
		return r;
	len = ((__u32)hdr[1] << 24) | ((__u32)hdr[2] << 16) |
	      ((__u32)hdr[3] << 8) | hdr[4];
	if (len > cap)
		return -EMSGSIZE;
	r = len ? xread_exact(vs, buf, len, deadline) : 0;
	if (r)
		return r;
	*t = hdr[0];
	return (long)len;
}

/* ---- record layer -------------------------------------------------------- */

static void make_nonce(__u8 out[12], const __u8 iv[12], __u64 seq)
{
	int i;

	memcpy(out, iv, 12);
	for (i = 0; i < 8; i++)
		out[11 - i] ^= (__u8)(seq >> (8 * i));
}

/* Encrypt n plaintext bytes (already at sbuf+5) as ONE application_data
 * record and write it fully to the TCP socket. */
static int rec_send(struct guard_sock *gs, const struct uk_file *f,
		    size_t n, __u8 inner_type, const char *what)
{
	__u8 nonce[12];
	size_t ctlen = n + 1 + REC_TAG;
	__u8 *h = gs->sbuf;
	int r;

	h[5 + n] = inner_type;
	h[0] = CT_APPDATA;
	h[1] = 0x03;
	h[2] = 0x03;
	h[3] = (__u8)(ctlen >> 8);
	h[4] = (__u8)ctlen;
	make_nonce(nonce, gs->tx_iv, gs->tx_seq);
	r = mbedtls_gcm_crypt_and_tag(&gs->tx, MBEDTLS_GCM_ENCRYPT, n + 1,
				      nonce, sizeof(nonce), h, REC_HDR,
				      h + REC_HDR, h + REC_HDR, REC_TAG,
				      h + REC_HDR + n + 1);
	if (r) {
		printf("MTLSGUARD: [c%u] GCM encrypt failed (%d)\n", gs->id, r);
		return -EIO;
	}
	r = orig_write_all(gs, f, h, REC_HDR + ctlen,
			   ukplat_monotonic_clock() + IO_TIMEOUT_NS);
	printf("MTLSGUARD: [c%u] tx record seq=%llu %s plaintext=%lu wire=%lu (outer 0x17, inner 0x%02x)%s t=%s\n",
	       gs->id, (unsigned long long)gs->tx_seq, what, (unsigned long)n,
	       (unsigned long)(REC_HDR + ctlen), inner_type,
	       r ? " WRITE FAILED" : "", mono());
	gs->tx_seq++;
	return r;
}

static ssize_t guard_sendv(const struct uk_file *f, const struct iovec *iov,
			   size_t iovcnt)
{
	struct guard_sock *gs = posix_sock_get_driver(f)->private;
	size_t n = 0, i;
	int r;

	if (gs->err)
		return gs->err;
	for (i = 0; i < iovcnt && n < REC_MAX_PLAIN; i++) {
		size_t take = MIN(iov[i].iov_len, REC_MAX_PLAIN - n);

		memcpy(gs->sbuf + REC_HDR + n, iov[i].iov_base, take);
		n += take;
	}
	if (n == 0)
		return 0;
	r = rec_send(gs, f, n, CT_APPDATA, "appdata");
	if (r) {
		gs->err = r;
		return r;
	}
	return (ssize_t)n;
}

/* Decrypt the complete record in rbuf. Returns 0 or a negative errno. */
static int rec_open(struct guard_sock *gs)
{
	size_t ctlen = ((size_t)gs->rbuf[3] << 8) | gs->rbuf[4];
	size_t n = ctlen - REC_TAG, i;
	__u8 nonce[12];
	__u8 inner;
	int r;

	make_nonce(nonce, gs->rx_iv, gs->rx_seq);
	r = mbedtls_gcm_auth_decrypt(&gs->rx, n, nonce, sizeof(nonce),
				     gs->rbuf, REC_HDR, gs->rbuf + REC_HDR + n,
				     REC_TAG, gs->rbuf + REC_HDR, gs->pbuf);
	gs->rlen = 0;
	if (r) {
		printf("MTLSGUARD: [c%u] rx record seq=%llu AUTH FAILED (%d) -> fail closed\n",
		       gs->id, (unsigned long long)gs->rx_seq, r);
		return -EBADMSG;
	}
	/* Strip zero padding; the last non-zero byte is the inner type. */
	i = n;
	while (i > 0 && gs->pbuf[i - 1] == 0)
		i--;
	if (i == 0) {
		printf("MTLSGUARD: [c%u] rx record seq=%llu has no content type -> fail closed\n",
		       gs->id, (unsigned long long)gs->rx_seq);
		return -EPROTO;
	}
	inner = gs->pbuf[i - 1];
	printf("MTLSGUARD: [c%u] rx record seq=%llu wire=%lu inner=0x%02x plaintext=%lu t=%s\n",
	       gs->id, (unsigned long long)gs->rx_seq,
	       (unsigned long)(REC_HDR + ctlen), inner, (unsigned long)(i - 1),
	       mono());
	gs->rx_seq++;
	switch (inner) {
	case CT_APPDATA:
		gs->poff = 0;
		gs->plen = i - 1;
		return 0;
	case CT_ALERT:
		if (i - 1 == 2 && gs->pbuf[0] == 1 && gs->pbuf[1] == 0) {
			printf("MTLSGUARD: [c%u] rx close_notify -> EOF\n", gs->id);
			gs->rx_eof = 1;
			return 0;
		}
		printf("MTLSGUARD: [c%u] rx alert level=%u desc=%u -> fail closed\n",
		       gs->id, gs->pbuf[0], i > 2 ? gs->pbuf[1] : 0);
		return -ECONNRESET;
	case CT_HANDSHAKE:
		printf("MTLSGUARD: [c%u] rx post-handshake handshake message type %u (%s) -> unsupported, fail closed\n",
		       gs->id, gs->pbuf[0],
		       gs->pbuf[0] == 4 ? "NewSessionTicket" :
		       gs->pbuf[0] == 24 ? "KeyUpdate" : "other");
		return -EPROTO;
	default:
		printf("MTLSGUARD: [c%u] rx inner type 0x%02x -> fail closed\n",
		       gs->id, inner);
		return -EPROTO;
	}
}

static size_t copy_out(struct guard_sock *gs, const struct iovec *iov,
		       size_t iovcnt)
{
	size_t done = 0, i;

	for (i = 0; i < iovcnt && gs->poff < gs->plen; i++) {
		size_t take = MIN(iov[i].iov_len, gs->plen - gs->poff);

		memcpy(iov[i].iov_base, gs->pbuf + gs->poff, take);
		gs->poff += take;
		done += take;
	}
	return done;
}

/* Non-blocking at the record level: returns -EAGAIN when lwIP has no more
 * bytes; the posix-fdio / posix-socket loop then polls POLLIN and retries. */
static ssize_t guard_recvv(const struct uk_file *f, const struct iovec *iov,
			   size_t iovcnt)
{
	struct guard_sock *gs = posix_sock_get_driver(f)->private;
	ssize_t r;
	int rc;

	for (;;) {
		if (gs->poff < gs->plen)
			return (ssize_t)copy_out(gs, iov, iovcnt);
		if (gs->err)
			return gs->err;
		if (gs->rx_eof)
			return 0;
		{
			size_t need = REC_HDR;
			struct iovec riov;

			if (gs->rlen >= REC_HDR) {
				size_t ctlen = ((size_t)gs->rbuf[3] << 8) |
					       gs->rbuf[4];

				if (gs->rbuf[0] != CT_APPDATA ||
				    gs->rbuf[1] != 0x03 || gs->rbuf[2] != 0x03 ||
				    ctlen < REC_TAG + 1 || ctlen > REC_MAX_CT) {
					printf("MTLSGUARD: [c%u] rx bad record header %02x %02x%02x len=%lu -> fail closed\n",
					       gs->id, gs->rbuf[0], gs->rbuf[1],
					       gs->rbuf[2], (unsigned long)ctlen);
					gs->err = -EPROTO;
					continue;
				}
				need = REC_HDR + ctlen;
				if (gs->rlen == need) {
					rc = rec_open(gs);
					if (rc)
						gs->err = rc;
					continue;
				}
			}
			/* Read exactly what the current record still needs. */
			riov.iov_base = gs->rbuf + gs->rlen;
			riov.iov_len = need - gs->rlen;
			r = gs->orig->ops->read(f, &riov, 1);
			if (r < 0)
				return r; /* incl. -EAGAIN */
			if (r == 0) {
				printf("MTLSGUARD: [c%u] TCP EOF %s close_notify -> %s\n",
				       gs->id, gs->rlen ? "mid-record," : "without",
				       gs->rlen ? "fail closed" : "EOF");
				if (gs->rlen)
					gs->err = -EPROTO;
				else
					gs->rx_eof = 1;
				continue;
			}
			gs->rlen += (size_t)r;
		}
	}
}

/* ---- guard driver ops ---------------------------------------------------- */

#define GS(f) ((struct guard_sock *)posix_sock_get_driver(f)->private)

static ssize_t g_read(posix_sock *f, const struct iovec *iov, size_t n)
{
	return guard_recvv(f, iov, n);
}

static ssize_t g_write(posix_sock *f, const struct iovec *iov, size_t n)
{
	return guard_sendv(f, iov, n);
}

static ssize_t g_recvfrom(posix_sock *f, void *restrict buf, size_t len,
			  int flags, struct sockaddr *from,
			  socklen_t *restrict fromlen)
{
	struct iovec iov = { .iov_base = buf, .iov_len = len };

	if (flags & MSG_PEEK)
		return -EOPNOTSUPP;
	if (from && fromlen)
		*fromlen = 0;
	return guard_recvv(f, &iov, 1);
}

static ssize_t g_recvmsg(posix_sock *f, struct msghdr *msg, int flags)
{
	if (flags & MSG_PEEK)
		return -EOPNOTSUPP;
	msg->msg_controllen = 0;
	msg->msg_flags = 0;
	msg->msg_namelen = 0;
	return guard_recvv(f, msg->msg_iov, msg->msg_iovlen);
}

static ssize_t g_sendto(posix_sock *f, const void *buf, size_t len,
			int flags __unused, const struct sockaddr *dst,
			socklen_t dstlen)
{
	struct iovec iov = { .iov_base = (void *)buf, .iov_len = len };

	if (dst || dstlen)
		return -EISCONN;
	return guard_sendv(f, &iov, 1);
}

static ssize_t g_sendmsg(posix_sock *f, const struct msghdr *msg,
			 int flags __unused)
{
	if (msg->msg_name || msg->msg_controllen)
		return -EOPNOTSUPP;
	return guard_sendv(f, msg->msg_iov, msg->msg_iovlen);
}

static void send_close_notify(struct guard_sock *gs, const struct uk_file *f)
{
	if (gs->err)
		return;
	gs->sbuf[REC_HDR] = 1;     /* warning */
	gs->sbuf[REC_HDR + 1] = 0; /* close_notify */
	if (rec_send(gs, f, 2, CT_ALERT, "close_notify"))
		gs->err = -EPIPE;
	else
		gs->err = -EPIPE; /* nothing may be sent after close_notify */
}

static int g_shutdown(posix_sock *f, int how)
{
	struct guard_sock *gs = GS(f);

	if (how == SHUT_WR || how == SHUT_RDWR)
		send_close_notify(gs, f);
	return gs->orig->ops->shutdown(f, how);
}

static int g_close(posix_sock *f)
{
	struct guard_sock *gs = GS(f);
	struct posix_socket_driver *orig = gs->orig;

	printf("MTLSGUARD: [c%u] close: tx records=%llu rx records=%llu\n",
	       gs->id, (unsigned long long)gs->tx_seq,
	       (unsigned long long)gs->rx_seq);
	send_close_notify(gs, f);
	/* Restore the original driver before freeing: socket_release() frees
	 * the socket with node.driver->allocator after this returns. */
	posix_sock_get_node(f)->driver = orig;
	mbedtls_gcm_free(&gs->tx);
	mbedtls_gcm_free(&gs->rx);
	mbedtls_platform_zeroize(gs, sizeof(*gs));
	uk_free(orig->allocator, gs);
	return orig->ops->close(f);
}

/* Plain forwarding to lwIP for everything the record layer does not touch. */
static void *g_accept4(posix_sock *f, struct sockaddr *restrict a,
		       socklen_t *restrict l, int fl)
{ return GS(f)->orig->ops->accept4(f, a, l, fl); }
static int g_bind(posix_sock *f, const struct sockaddr *a, socklen_t l)
{ return GS(f)->orig->ops->bind(f, a, l); }
static int g_getpeername(posix_sock *f, struct sockaddr *restrict a,
			 socklen_t *restrict l)
{ return GS(f)->orig->ops->getpeername(f, a, l); }
static int g_getsockname(posix_sock *f, struct sockaddr *restrict a,
			 socklen_t *restrict l)
{ return GS(f)->orig->ops->getsockname(f, a, l); }
static int g_getsockopt(posix_sock *f, int lv, int o, void *restrict v,
			socklen_t *restrict l)
{ return GS(f)->orig->ops->getsockopt(f, lv, o, v, l); }
static int g_setsockopt(posix_sock *f, int lv, int o, const void *v,
			socklen_t l)
{ return GS(f)->orig->ops->setsockopt(f, lv, o, v, l); }
static int g_connect(posix_sock *f __unused, const struct sockaddr *a __unused,
		     socklen_t l __unused)
{ return -EISCONN; }
static int g_listen(posix_sock *f __unused, int b __unused)
{ return -EOPNOTSUPP; }
static int g_ioctl(posix_sock *f, int req, void *argp)
{ return GS(f)->orig->ops->ioctl(f, req, argp); }
static void g_poll_setup(posix_sock *f)
{ GS(f)->orig->ops->poll_setup(f); }

static const struct posix_socket_ops guard_ops = {
	.accept4 = g_accept4,
	.bind = g_bind,
	.shutdown = g_shutdown,
	.getpeername = g_getpeername,
	.getsockname = g_getsockname,
	.getsockopt = g_getsockopt,
	.setsockopt = g_setsockopt,
	.connect = g_connect,
	.listen = g_listen,
	.recvfrom = g_recvfrom,
	.recvmsg = g_recvmsg,
	.sendmsg = g_sendmsg,
	.sendto = g_sendto,
	.write = g_write,
	.read = g_read,
	.close = g_close,
	.ioctl = g_ioctl,
	.poll_setup = g_poll_setup,
};

/* ---- handshake relay + install ------------------------------------------ */

static __u64 be64(const __u8 *p)
{
	__u64 v = 0;
	int i;

	for (i = 0; i < 8; i++)
		v = (v << 8) | p[i];
	return v;
}

static int install(const struct uk_file *tcp, unsigned int id,
		   const __u8 *p, long len)
{
	struct posix_socket_driver *orig = posix_sock_get_driver(tcp);
	struct guard_sock *gs;
	size_t idlen;
	int r;

	/* suite(2) + 2 x (seq 8 + keylen 1 + key 16 + iv 12) + idlen(2) */
	if (len < 2 + 2 * 37 + 2 || p[0] != 0x13 || p[1] != 0x01 ||
	    p[10] != 16 || p[10 + 1 + 16 + 12 + 8] != 16) {
		printf("MTLSGUARD: [c%u] malformed SECRETS (len=%ld)\n", id, len);
		return -EPROTO;
	}
	gs = uk_calloc(orig->allocator, 1, sizeof(*gs));
	if (!gs)
		return -ENOMEM;
	{
		struct posix_socket_driver tmpl = {
			.family = AF_INET,
			.libname = "mtlsguard",
			.ops = &guard_ops,
			.allocator = orig->allocator,
			.private = gs,
		};
		memcpy(&gs->drv, &tmpl, sizeof(tmpl));
	}
	gs->orig = orig;
	gs->id = id;
	gs->tx_seq = be64(p + 2);
	memcpy(gs->tx_iv, p + 2 + 8 + 1 + 16, 12);
	gs->rx_seq = be64(p + 2 + 37);
	memcpy(gs->rx_iv, p + 2 + 37 + 8 + 1 + 16, 12);
	mbedtls_gcm_init(&gs->tx);
	mbedtls_gcm_init(&gs->rx);
	r = mbedtls_gcm_setkey(&gs->tx, MBEDTLS_CIPHER_ID_AES,
			       p + 2 + 8 + 1, 128);
	if (!r)
		r = mbedtls_gcm_setkey(&gs->rx, MBEDTLS_CIPHER_ID_AES,
				       p + 2 + 37 + 8 + 1, 128);
	if (r) {
		printf("MTLSGUARD: [c%u] gcm_setkey failed (%d)\n", id, r);
		mbedtls_gcm_free(&gs->tx);
		mbedtls_gcm_free(&gs->rx);
		mbedtls_platform_zeroize(gs, sizeof(*gs));
		uk_free(orig->allocator, gs);
		return -EIO;
	}
	idlen = ((size_t)p[2 + 74] << 8) | p[2 + 74 + 1];
	if (idlen >= sizeof(gs->peer) || (long)(2 + 74 + 2 + idlen) > len)
		idlen = 0;
	memcpy(gs->peer, p + 2 + 74 + 2, idlen);
	gs->peer[idlen] = '\0';
	/* THE SWAP: this one socket now dispatches through the guard ops. */
	posix_sock_get_node(tcp)->driver = &gs->drv;
	printf("MTLSGUARD: [c%u] record layer installed: TLS_AES_128_GCM_SHA256 tx_seq=%llu rx_seq=%llu, peer identity (from host) %s; socket driver lwip -> mtlsguard t=%s\n",
	       id, (unsigned long long)gs->tx_seq,
	       (unsigned long long)gs->rx_seq, gs->peer, mono());
	return 0;
}

static __u8 fbuf[MAX_FRAME];

/* Read exactly ONE TLS record (5-byte header + body) from the TCP peer. */
static long read_one_record(const struct uk_file *tcp, __u8 *buf,
			    __nsec deadline)
{
	size_t len;
	ssize_t r;
	int rc;

	r = xread(tcp, buf, 1, deadline);
	if (r <= 0)
		return r; /* 0 = EOF before any byte */
	rc = xread_exact(tcp, buf + 1, REC_HDR - 1, deadline);
	if (rc)
		return rc;
	len = ((size_t)buf[3] << 8) | buf[4];
	if (len > REC_MAX_CT)
		return -EPROTO;
	rc = len ? xread_exact(tcp, buf + REC_HDR, len, deadline) : 0;
	if (rc)
		return rc;
	return (long)(REC_HDR + len);
}

static int handshake(const struct uk_file *tcp, __u8 open_type,
		     const __u8 *open, __u32 open_len, unsigned int id)
{
	struct sockaddr_vm svm;
	struct uk_file *vs;
	__nsec deadline = ukplat_monotonic_clock() + HANDSHAKE_TIMEOUT_NS;
	unsigned int n_to = 0, n_need = 0, b_to = 0, b_from = 0;
	int r, ret;

	vs = uk_socket_create(AF_VSOCK, SOCK_STREAM, 0);
	if (PTRISERR(vs)) {
		printf("MTLSGUARD: [c%u] vsock socket failed (%d)\n", id,
		       PTR2ERR(vs));
		return -ECONNABORTED;
	}
	memset(&svm, 0, sizeof(svm));
	svm.svm_family = AF_VSOCK;
	svm.svm_cid = VMADDR_CID_HOST;
	svm.svm_port = agent_port;
	r = posix_socket_connect(vs, (struct sockaddr *)&svm, sizeof(svm));
	if (r == -EINPROGRESS) {
		int soerr = 0;
		socklen_t sl = sizeof(soerr);
		uk_pollevent ev = uk_file_poll_until(vs,
			UKFD_POLLOUT | UKFD_POLL_ALWAYS, deadline);

		posix_socket_getsockopt(vs, SOL_SOCKET, SO_ERROR, &soerr, &sl);
		r = (ev & UKFD_POLLOUT) && !soerr ? 0 :
		    (soerr ? (soerr < 0 ? soerr : -soerr) : -ETIMEDOUT);
	}
	if (r) {
		printf("MTLSGUARD: [c%u] host relay unreachable on vsock cid 2 port %u (%d) -> fail closed t=%s\n",
		       id, (unsigned int)agent_port, r, mono());
		uk_file_release(vs);
		return -ECONNABORTED;
	}
	printf("MTLSGUARD: [c%u] vsock connected to host relay (cid 2 port %u) t=%s\n",
	       id, (unsigned int)agent_port, mono());

	if (frame_send(vs, id, open_type, open, open_len, deadline)) {
		ret = -ECONNABORTED;
		goto out;
	}
	for (;;) {
		__u8 t = 0;
		long len = frame_recv(vs, &t, fbuf, sizeof(fbuf), deadline);

		if (len < 0) {
			printf("MTLSGUARD: [c%u] vsock recv failed (%ld) -> fail closed\n",
			       id, len);
			ret = -ECONNABORTED;
			goto out;
		}
		printf("MTLSGUARD: [c%u] vsock rx %s payload=%ld wire=%ld t=%s\n",
		       id, tname(t), len, len + 5, mono());
		switch (t) {
		case T_TO_PEER:
			n_to++;
			b_to += (unsigned int)len;
			r = xwrite_all(tcp, fbuf, (size_t)len, deadline);
			printf("MTLSGUARD: [c%u] tcp tx %ld handshake bytes to peer%s\n",
			       id, len, r ? " FAILED" : "");
			if (r) {
				ret = -ECONNABORTED;
				goto out;
			}
			break;
		case T_NEED_PEER: {
			ssize_t got;

			n_need++;
			got = read_one_record(tcp, fbuf, deadline);
			if (got < 0) {
				printf("MTLSGUARD: [c%u] tcp rx failed (%ld)\n",
				       id, (long)got);
				ret = -ECONNABORTED;
				goto out;
			}
			b_from += (unsigned int)got;
			printf("MTLSGUARD: [c%u] tcp rx one handshake record from peer: %ld bytes (type 0x%02x)\n",
			       id, (long)got, got ? fbuf[0] : 0);
			if (frame_send(vs, id, T_FROM_PEER, fbuf, (__u32)got,
				       deadline)) {
				ret = -ECONNABORTED;
				goto out;
			}
			break;
		}
		case T_SECRETS:
			ret = install(tcp, id, fbuf, len);
			mbedtls_platform_zeroize(fbuf, (size_t)len);
			goto out;
		case T_DENY:
			fbuf[len < (long)sizeof(fbuf) ? len : (long)sizeof(fbuf) - 1] = 0;
			printf("MTLSGUARD: [c%u] host DENY: %s\n", id, fbuf);
			ret = -EACCES;
			goto out;
		case T_ERROR:
			fbuf[len < (long)sizeof(fbuf) ? len : (long)sizeof(fbuf) - 1] = 0;
			printf("MTLSGUARD: [c%u] host ERROR: %s\n", id, fbuf);
			ret = -ECONNABORTED;
			goto out;
		default:
			printf("MTLSGUARD: [c%u] unknown frame type 0x%02x -> fail closed\n",
			       id, t);
			ret = -EPROTO;
			goto out;
		}
	}
out:
	printf("MTLSGUARD: [c%u] vsock summary: rx TO_PEER x%u (%u B), NEED_PEER x%u; tx FROM_PEER %u B; closing vsock (agent leaves the data path) t=%s\n",
	       id, n_to, b_to, n_need, b_from, mono());
	uk_file_release(vs);
	return ret;
}

static void abort_tcp(const struct uk_file *sock)
{
	(void)posix_socket_shutdown(sock, SHUT_RDWR);
}

static void self_test_once(void)
{
	static int done;

	if (done)
		return;
	done = 1;
	printf("MTLSGUARD: Mbed TLS 3.6.7 AES-GCM known-answer self test:\n");
	printf("MTLSGUARD: gcm self test result=%d, AES-NI in use=%s, mesh=\"%s\", mesh_in=\"%s\", agent vsock port=%u\n",
	       mbedtls_gcm_self_test(1),
	       mbedtls_aesni_has_support(MBEDTLS_AESNI_AES) ? "yes" : "no",
	       mesh ? mesh : "", mesh_in ? mesh_in : "",
	       (unsigned int)agent_port);
}

int uk_socket_connect_hook(const struct uk_file *sock,
			   const struct sockaddr *addr, socklen_t addr_len,
			   int blocking, int ret)
{
	const struct sockaddr_in *sin = (const struct sockaddr_in *)addr;
	socklen_t tl = sizeof(int);
	int type = 0, r;
	unsigned int id;
	__nsec t_hold;
	char dst[24];

	self_test_once();
	if (addr->sa_family != AF_INET || addr_len < sizeof(*sin))
		return ret;
	if (posix_socket_getsockopt(sock, SOL_SOCKET, SO_TYPE, &type, &tl) ||
	    type != SOCK_STREAM)
		return ret;
	fmt_dst(dst, sizeof(dst), sin);
	if (!is_mesh(mesh, sin)) {
		printf("MTLSGUARD: connect %s: not a mesh destination -> pass-through, driver untouched t=%s\n",
		       dst, mono());
		return ret;
	}
	if (!blocking) {
		printf("MTLSGUARD: connect %s: NON-BLOCKING mesh connect -> refused with EOPNOTSUPP (readiness hold not implemented in this spike) t=%s\n",
		       dst, mono());
		abort_tcp(sock);
		return -EOPNOTSUPP;
	}
	id = ++conn_seq;
	t_hold = ukplat_monotonic_clock();
	printf("MTLSGUARD: [c%u] connect %s: mesh destination; TCP is up -> HOLD connect() until the host handshake completes t=%s\n",
	       id, dst, mono());
	{
		__u8 open[7];

		open[0] = 4;
		memcpy(open + 1, &sin->sin_addr.s_addr, 4); /* network order */
		memcpy(open + 5, &sin->sin_port, 2);        /* network order */
		uk_file_wlock(sock);
		r = handshake(sock, T_OPEN, open, sizeof(open), id);
		uk_file_wunlock(sock);
	}
	if (r) {
		abort_tcp(sock);
		printf("MTLSGUARD: [c%u] connect %s: FAIL closed, connect() returns %d, TCP shut down, no application data sent (held %llu us) t=%s\n",
		       id, dst, r,
		       (unsigned long long)(ukplat_monotonic_clock() - t_hold) / 1000ull,
		       mono());
		return r;
	}
	printf("MTLSGUARD: [c%u] connect %s: RELEASE connect() = 0 after %llu us hold t=%s\n",
	       id, dst,
	       (unsigned long long)(ukplat_monotonic_clock() - t_hold) / 1000ull,
	       mono());
	return 0;
}

int uk_socket_accept_hook(const struct uk_file *listener __unused,
			  const struct uk_file *sock, int blocking, int flags)
{
	struct sockaddr_in loc, rem;
	socklen_t ll = sizeof(loc), rl = sizeof(rem);
	char lstr[24], rstr[24];
	__u8 acc[13];
	unsigned int id;
	__nsec t_hold;
	int r;

	self_test_once();
	memset(&loc, 0, sizeof(loc));
	memset(&rem, 0, sizeof(rem));
	if (posix_socket_getsockname(sock, (struct sockaddr *)&loc, &ll) ||
	    loc.sin_family != AF_INET ||
	    posix_socket_getpeername(sock, (struct sockaddr *)&rem, &rl))
		return 0; /* not an AF_INET stream we can classify: untouched */
	fmt_dst(lstr, sizeof(lstr), &loc);
	fmt_dst(rstr, sizeof(rstr), &rem);
	if (!is_mesh(mesh_in, &loc)) {
		printf("MTLSGUARD: accept %s <- %s: not a mesh listener -> pass-through t=%s\n",
		       lstr, rstr, mono());
		return 0;
	}
	if (!blocking || (flags & SOCK_NONBLOCK)) {
		printf("MTLSGUARD: accept %s <- %s: NON-BLOCKING mesh accept -> refused with EOPNOTSUPP (spike gap)\n",
		       lstr, rstr);
		return -EOPNOTSUPP;
	}
	id = ++conn_seq;
	t_hold = ukplat_monotonic_clock();
	printf("MTLSGUARD: [c%u] accept %s <- %s: mesh listener -> HOLD the new socket (no fd yet) until the host handshake completes t=%s\n",
	       id, lstr, rstr, mono());
	acc[0] = 4;
	memcpy(acc + 1, &loc.sin_addr.s_addr, 4);
	memcpy(acc + 5, &loc.sin_port, 2);
	memcpy(acc + 7, &rem.sin_addr.s_addr, 4);
	memcpy(acc + 11, &rem.sin_port, 2);
	uk_file_wlock(sock);
	r = handshake(sock, T_ACCEPT, acc, sizeof(acc), id);
	uk_file_wunlock(sock);
	if (r) {
		abort_tcp(sock);
		printf("MTLSGUARD: [c%u] accept %s <- %s: FAIL closed, connection dropped, accept() returns %d (held %llu us) t=%s\n",
		       id, lstr, rstr, r,
		       (unsigned long long)(ukplat_monotonic_clock() - t_hold) / 1000ull,
		       mono());
		return r == -EACCES ? -ECONNABORTED : r;
	}
	printf("MTLSGUARD: [c%u] accept %s <- %s: RELEASE, accept() gets the socket after %llu us hold t=%s\n",
	       id, lstr, rstr,
	       (unsigned long long)(ukplat_monotonic_clock() - t_hold) / 1000ull,
	       mono());
	return 0;
}
