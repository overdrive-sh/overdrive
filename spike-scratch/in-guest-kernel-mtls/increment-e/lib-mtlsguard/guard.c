/* SPDX-License-Identifier: BSD-3-Clause */
/*
 * Spike B2 (Overdrive GH #303, increment-e): in-guest transparent mTLS for a
 * Unikraft unikernel, with the private key and the TLS handshake on the host,
 * extended to non-blocking sockets with poll/epoll readiness, post-handshake
 * NewSessionTicket and TLS 1.3 KeyUpdate. THROWAWAY PROBE CODE.
 *
 * Built on Spike B (increment-d). What is new:
 *
 *  1. GUARD THREAD. Every mesh connection (connect() or accept()) gets its own
 *     guard thread and a per-socket guard driver installed IMMEDIATELY (state
 *     HANDSHAKING). The thread waits for TCP, runs the Spike B vsock relay
 *     (lock-step, one TLS record per NEED_PEER), installs the record layer,
 *     and then stays on as the connection's RX pump. A non-blocking connect()
 *     returns EINPROGRESS at once; a blocking connect()/accept() waits for the
 *     thread's HSDONE (the Spike B hold, now implemented with the same thread).
 *
 *  2. READINESS. lib-lwip (patch 0003) hands lwIP's own readiness for a socket
 *     that carries a foreign driver to lwip_posix_socket_events_hook() and
 *     assigns what the hook returns. The hook mirrors lwIP's raw readiness into
 *     a private pollqueue (rawq) the guard thread waits on, and returns the
 *     APPLICATION readiness, which is computed only from guard state:
 *       HANDSHAKING: nothing (no EPOLLOUT, no EPOLLIN);
 *       FAILED:      EPOLLERR|EPOLLHUP (SO_ERROR gives the errno);
 *       ESTABLISHED: EPOLLIN iff decrypted plaintext is buffered, or EOF/error;
 *                    EPOLLOUT iff the TX ciphertext buffer has room for a full
 *                    record. A partial ciphertext record never raises EPOLLIN:
 *                    only the RX pump decrypts, one complete record at a time,
 *                    and the application's read() only copies plaintext out.
 *     The same function is published (uk_file_event_assign) after every guard
 *     state change, so edge-triggered epoll sees one rising edge per newly
 *     decrypted record, and level-triggered poll/epoll keep reporting EPOLLIN
 *     while plaintext is buffered even though lwIP's own EPOLLIN is clear.
 *
 *  3. POST-HANDSHAKE MESSAGES. Inner-type 0x16 records are reassembled into
 *     handshake messages (a message may span records; a record may carry
 *     several). NewSessionTicket (4) is discarded. KeyUpdate (24) advances the
 *     receive keys and, if update_requested, queues a KeyUpdate of our own under
 *     the current keys and then advances the transmit keys. Anything else
 *     (and a KeyUpdate that is not at a record boundary, or application data
 *     inside a fragmented handshake message) fails the connection closed with
 *     an unexpected_message alert.
 *
 *  4. KEY SCHEDULE. SECRETS v2 carries the TLS 1.3 application traffic secret
 *     of each direction (the host captured them with rustls' KeyLog). The
 *     guest derives key and IV with HKDF-Expand-Label (Mbed TLS HKDF over
 *     SHA-256) and cross-checks them against rustls' key/IV. The SVID private
 *     key still never crosses vsock; the traffic secrets now do.
 *
 * Out of scope (spike): multi-threaded applications (the design assumes the
 * cooperative single-vCPU scheduler), session resumption, TCP Fast Open,
 * sendfile/splice, app-elfloader.
 */
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/epoll.h>
#include <sys/ioctl.h>
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
#include <uk/lwip_readiness.h>
#include <uk/mutex.h>
#include <uk/plat/time.h>
#include <uk/posix-fd.h>
#include <uk/sched.h>
#include <uk/socket.h>
#include <uk/socket_driver.h>

#include <mbedtls/gcm.h>
#include <mbedtls/hkdf.h>
#include <mbedtls/md.h>
#include <mbedtls/platform_util.h>
#include <mbedtls/sha256.h>
#include "aesni.h" /* Mbed TLS internal: mbedtls_aesni_has_support() */

#ifndef AF_VSOCK
#define AF_VSOCK 40
#endif
#ifndef FIONREAD
#define FIONREAD 0x541B
#endif

/* ---- configuration (library parameters) --------------------------------- */

static char *mesh;
UK_LIBPARAM_PARAM(mesh, charp, "mesh destinations ip:port[-port],...");
static char *mesh_in;
UK_LIBPARAM_PARAM(mesh_in, charp, "mesh listeners ip:port[-port],...");
static __u32 agent_port = 7100;
UK_LIBPARAM_PARAM(agent_port, __u32, "host relay vsock port");

#define HANDSHAKE_TIMEOUT_NS (10ULL * 1000000000ULL)
#define CLOSE_FLUSH_NS       (2ULL * 1000000000ULL)
#define CLOSE_JOIN_NS        (4ULL * 1000000000ULL)

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

/* ---- TLS 1.3 record layer constants -------------------------------------- */

#define REC_HDR       5
#define REC_TAG       16
#define REC_MAX_PLAIN 16384
#define REC_MAX_CT    (REC_MAX_PLAIN + 256)
#define REC_OVERHEAD  (REC_HDR + 1 + REC_TAG)
#define CT_APPDATA    0x17
#define CT_ALERT      0x15
#define CT_HANDSHAKE  0x16
#define HS_NST        4
#define HS_KEYUPDATE  24
#define HS_MAX        8192
/* TX ciphertext buffer: two full records plus a reserve that application
 * writes never use, so a KeyUpdate / alert / close_notify always fits. */
#define TBUF_RESERVE  512
#define TBUF_SIZE     (2 * (REC_OVERHEAD + REC_MAX_PLAIN) + TBUF_RESERVE)

#define ALERT_UNEXPECTED_MESSAGE 10
#define ALERT_BAD_RECORD_MAC     20
#define ALERT_DECODE_ERROR       50

/* Guard-internal pollqueue bits (above every EPOLL* flag the app can see). */
#define GS_EV_KICK     (1U << 24)
#define GS_EV_HSDONE   (1U << 25)
#define GS_EV_INTERNAL (GS_EV_KICK | GS_EV_HSDONE)

enum { GS_HANDSHAKING = 0, GS_ESTABLISHED = 1, GS_FAILED = 2 };
enum { ROLE_CONNECT = 0, ROLE_ACCEPT = 1 };

struct dir_keys {
	mbedtls_gcm_context gcm;
	__u8 secret[32];
	__u8 iv[12];
	__u64 seq;
	unsigned int gen;       /* KeyUpdates applied */
};

struct guard_sock {
	struct posix_socket_driver drv;  /* per-socket driver: .private = this */
	struct posix_socket_driver *orig; /* lwIP's AF_INET driver */
	const struct uk_file *sock;
	unsigned int id;
	int role;
	char desc[64];
	__u8 open_type;
	__u8 open[13];
	__u32 open_len;
	int tcp_connecting;
	int blocking;

	volatile int state;
	int err;                    /* handshake failure, negative errno */
	int so_error_reported;
	volatile int closing;
	volatile int thread_done;
	const struct uk_file *vs;   /* vsock to the host relay during the handshake */

	struct uk_mutex lock;       /* tx/rx buffers, keys, lwIP I/O */
	struct uk_pollq rawq;       /* lwIP's raw readiness + GS_EV_* */
	volatile unsigned int lwip_ev;
	__nsec t_start, t_tcp_up, t_installed, t_raw_out_first;
	unsigned long hs_eagain_writes, hs_eagain_reads;

	struct dir_keys tx, rx;
	int keys_ready;

	size_t rlen;                /* ciphertext bytes of the current record */
	size_t poff, plen;          /* decrypted plaintext not yet read by the app */
	size_t hslen;               /* buffered post-handshake handshake bytes */
	int rx_eof, rx_err;
	unsigned long spin;

	size_t toff, tlen;          /* queued ciphertext [toff, tlen) */
	int tx_closed;              /* no more application data (negative errno) */
	int tx_err;                 /* lwIP write failed: stop flushing */
	int shut_req, shut_how, shut_done;

	unsigned long nst, ku_rx, ku_tx, rx_app_records, tx_app_records;
	char peer[128];

	__u8 rbuf[REC_HDR + REC_MAX_CT];
	__u8 pbuf[REC_MAX_CT];
	__u8 hsbuf[HS_MAX];
	__u8 tbuf[TBUF_SIZE];
};

static unsigned int conn_seq;

/* ---- logging helpers ----------------------------------------------------- */

static const char *fmt_ns(char *buf, size_t n, __nsec ns)
{
	snprintf(buf, n, "%llu.%06llu", (unsigned long long)ns / 1000000000ull,
		 ((unsigned long long)ns % 1000000000ull) / 1000ull);
	return buf;
}

#define MONO() fmt_ns((char[32]){0}, 32, ukplat_monotonic_clock())

static const char *evstr(char *b, size_t n, unsigned int ev)
{
	snprintf(b, n, "%s%s%s%s%s%s",
		 ev == 0 ? "none" : "",
		 (ev & EPOLLIN) ? "IN " : "", (ev & EPOLLOUT) ? "OUT " : "",
		 (ev & EPOLLERR) ? "ERR " : "", (ev & EPOLLHUP) ? "HUP " : "",
		 (ev & EPOLLRDHUP) ? "RDHUP " : "");
	return b;
}

static void fmt_dst(char *out, size_t n, const struct sockaddr_in *sin)
{
	__u32 a = ntohl(sin->sin_addr.s_addr);

	snprintf(out, n, "%u.%u.%u.%u:%u", (a >> 24) & 0xff, (a >> 16) & 0xff,
		 (a >> 8) & 0xff, a & 0xff, (unsigned int)ntohs(sin->sin_port));
}

/* ---- mesh destination matching (Spike B) --------------------------------- */

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

/* ---- key schedule: HKDF-Expand-Label (RFC 8446 7.1) over SHA-256 ---------- */

static int hkdf_expand_label(const __u8 secret[32], const char *label,
			     __u8 *out, size_t outlen)
{
	__u8 info[2 + 1 + 6 + 32 + 1];
	size_t ll = strlen(label);

	UK_ASSERT(ll <= 32);
	info[0] = (__u8)(outlen >> 8);
	info[1] = (__u8)outlen;
	info[2] = (__u8)(6 + ll);
	memcpy(info + 3, "tls13 ", 6);
	memcpy(info + 9, label, ll);
	info[9 + ll] = 0; /* empty context */
	return mbedtls_hkdf_expand(mbedtls_md_info_from_type(MBEDTLS_MD_SHA256),
				   secret, 32, info, 10 + ll, out, outlen);
}

/* Derive key + IV from `secret` and install them (sequence number untouched). */
static int keys_set(struct dir_keys *d, const __u8 secret[32])
{
	__u8 key[16];
	int r;

	r = hkdf_expand_label(secret, "key", key, sizeof(key));
	if (!r)
		r = hkdf_expand_label(secret, "iv", d->iv, sizeof(d->iv));
	if (!r) {
		mbedtls_gcm_free(&d->gcm);
		mbedtls_gcm_init(&d->gcm);
		r = mbedtls_gcm_setkey(&d->gcm, MBEDTLS_CIPHER_ID_AES, key, 128);
	}
	if (!r && d->secret != secret)
		memcpy(d->secret, secret, 32);
	mbedtls_platform_zeroize(key, sizeof(key));
	return r;
}

/* RFC 8446 7.2: application_traffic_secret_N+1, new key/IV, sequence 0. */
static int keys_update(struct dir_keys *d)
{
	__u8 next[32];
	int r = hkdf_expand_label(d->secret, "traffic upd", next, sizeof(next));

	if (!r)
		r = keys_set(d, next);
	mbedtls_platform_zeroize(next, sizeof(next));
	if (!r) {
		d->seq = 0;
		d->gen++;
	}
	return r;
}

static void self_test_once(void)
{
	/* RFC 8448 section 3: {server} application traffic secret -> key, iv. */
	static const __u8 s[32] = {
		0xa1, 0x1a, 0xf9, 0xf0, 0x55, 0x31, 0xf8, 0x56,
		0xad, 0x47, 0x11, 0x6b, 0x45, 0xa9, 0x50, 0x32,
		0x82, 0x04, 0xb4, 0xf4, 0x4b, 0xfb, 0x6b, 0x3a,
		0x4b, 0x4f, 0x1f, 0x3f, 0xcb, 0x63, 0x16, 0x43 };
	static const __u8 k[16] = {
		0x9f, 0x02, 0x28, 0x3b, 0x6c, 0x9c, 0x07, 0xef,
		0xc2, 0x6b, 0xb9, 0xf2, 0xac, 0x92, 0xe3, 0x56 };
	static const __u8 iv[12] = {
		0xcf, 0x78, 0x2b, 0x88, 0xdd, 0x83, 0x54, 0x9a,
		0xad, 0xf1, 0xe9, 0x84 };
	/* "traffic upd" of the same secret, computed off-guest with Python
	 * hmac/hashlib (RFC 8448 has no KeyUpdate vector). */
	static const __u8 upd[32] = {
		0x51, 0x92, 0x1b, 0x8a, 0xa3, 0x00, 0x19, 0x76,
		0xeb, 0x40, 0x1d, 0x0a, 0x43, 0x19, 0xa8, 0x51,
		0x64, 0x16, 0xa6, 0xc5, 0x60, 0x01, 0xa3, 0x57,
		0xe5, 0xd1, 0x62, 0x03, 0x1e, 0x84, 0xf9, 0x16 };
	static int done;
	__u8 ok[16], oiv[12], oupd[32];
	int kr, ir, ur;

	if (done)
		return;
	done = 1;
	kr = hkdf_expand_label(s, "key", ok, 16);
	ir = hkdf_expand_label(s, "iv", oiv, 12);
	ur = hkdf_expand_label(s, "traffic upd", oupd, 32);
	printf("MTLSGUARD: self test: Mbed TLS 3.6.7 gcm=%d sha256=%d AES-NI=%s; HKDF-Expand-Label RFC 8448 key=%s iv=%s, traffic upd (python vector)=%s\n",
	       mbedtls_gcm_self_test(0), mbedtls_sha256_self_test(0),
	       mbedtls_aesni_has_support(MBEDTLS_AESNI_AES) ? "yes" : "no",
	       (!kr && !memcmp(ok, k, 16)) ? "OK" : "FAIL",
	       (!ir && !memcmp(oiv, iv, 12)) ? "OK" : "FAIL",
	       (!ur && !memcmp(oupd, upd, 32)) ? "OK" : "FAIL");
	printf("MTLSGUARD: config mesh=\"%s\" mesh_in=\"%s\" agent vsock port=%u\n",
	       mesh ? mesh : "", mesh_in ? mesh_in : "",
	       (unsigned int)agent_port);
}

/* ---- readiness ----------------------------------------------------------- */

static const struct posix_socket_ops guard_ops;

static size_t tx_used(const struct guard_sock *gs)
{
	return gs->tlen - gs->toff;
}

static long tx_room(const struct guard_sock *gs)
{
	return (long)TBUF_SIZE - (long)tx_used(gs) - TBUF_RESERVE;
}

/* The readiness the APPLICATION sees: a pure function of guard state. */
static unsigned int app_events(const struct guard_sock *gs)
{
	unsigned int ev = 0;

	if (gs->state == GS_HANDSHAKING)
		return 0;
	if (gs->state == GS_FAILED)
		return EPOLLERR | EPOLLHUP;
	if (gs->poff < gs->plen || gs->rx_eof || gs->rx_err)
		ev |= EPOLLIN | EPOLLRDNORM;
	if (gs->rx_eof)
		ev |= EPOLLRDHUP;
	if (gs->rx_err || gs->tx_err)
		ev |= EPOLLERR;
	if (!gs->tx_closed && !gs->tx_err &&
	    tx_room(gs) >= REC_OVERHEAD + REC_MAX_PLAIN)
		ev |= EPOLLOUT | EPOLLWRNORM;
	if (gs->rx_eof && gs->shut_done)
		ev |= EPOLLHUP;
	return ev;
}

static void publish(struct guard_sock *gs)
{
	unsigned int ev = app_events(gs);
	unsigned int prev = uk_file_event_assign(gs->sock, ev);
	char a[48], b[48];

	if ((prev & ~(EPOLLRDNORM | EPOLLWRNORM)) !=
	    (ev & ~(EPOLLRDNORM | EPOLLWRNORM)))
		printf("MTLSGUARD: [c%u] app readiness %s-> %s(published by the guard) t=%s\n",
		       gs->id, evstr(a, sizeof(a), prev), evstr(b, sizeof(b), ev),
		       MONO());
}

static void kick(struct guard_sock *gs)
{
	uk_pollq_set(&gs->rawq, GS_EV_KICK);
}

/*
 * lib-lwip patch 0003: lwIP calls this for a socket whose driver is not its
 * own, with the readiness lwIP computed, and assigns what we return. Called
 * from lwIP's event callback with interrupts disabled (and from poll_setup):
 * no blocking, no locks, no printing.
 */
unsigned int lwip_posix_socket_events_hook(posix_sock *sock,
					   unsigned int lwip_events)
{
	struct posix_socket_driver *d = posix_sock_get_driver(sock);
	struct guard_sock *gs;
	unsigned int internal;

	if (d->ops != &guard_ops)
		return lwip_events; /* some other interposer: not ours */
	gs = d->private;
	gs->lwip_ev = lwip_events;
	if (gs->state == GS_HANDSHAKING && (lwip_events & EPOLLOUT) &&
	    !gs->t_raw_out_first)
		gs->t_raw_out_first = ukplat_monotonic_clock();
	internal = gs->rawq.events & GS_EV_INTERNAL;
	uk_pollq_assign(&gs->rawq, (lwip_events & ~GS_EV_INTERNAL) | internal);
	return app_events(gs);
}

/* Wait on lwIP's raw readiness (or a kick). 0 = woken, <0 = timeout/closing. */
static int raw_wait(struct guard_sock *gs, unsigned int mask, __nsec deadline)
{
	uk_pollevent ev;

	uk_pollq_clear(&gs->rawq, GS_EV_KICK);
	if (gs->closing)
		return -ECANCELED;
	ev = uk_pollq_poll_until(&gs->rawq, mask | GS_EV_KICK, deadline, NULL);
	if (gs->closing)
		return -ECANCELED;
	return ev ? 0 : -ETIMEDOUT;
}

/* ---- lwIP TCP I/O from the guard (serialized by gs->lock) ----------------- */

static ssize_t tcp_io(struct guard_sock *gs, int wr, void *buf, size_t n)
{
	struct iovec iov = { .iov_base = buf, .iov_len = n };
	ssize_t r;

	uk_mutex_lock(&gs->lock);
	r = wr ? gs->orig->ops->write(gs->sock, &iov, 1)
	       : gs->orig->ops->read(gs->sock, &iov, 1);
	uk_mutex_unlock(&gs->lock);
	return r;
}

static ssize_t tcp_read_some(struct guard_sock *gs, void *buf, size_t n,
			     __nsec deadline)
{
	for (;;) {
		ssize_t r = tcp_io(gs, 0, buf, n);
		int w;

		if (r != -EAGAIN)
			return r;
		w = raw_wait(gs, EPOLLIN | EPOLLRDHUP | EPOLLERR | EPOLLHUP,
			     deadline);
		if (w < 0)
			return w;
	}
}

static int tcp_read_exact(struct guard_sock *gs, void *buf, size_t n,
			  __nsec deadline)
{
	size_t got = 0;

	while (got < n) {
		ssize_t r = tcp_read_some(gs, (__u8 *)buf + got, n - got,
					  deadline);

		if (r < 0)
			return (int)r;
		if (r == 0)
			return -ECONNRESET;
		got += (size_t)r;
	}
	return 0;
}

static int tcp_write_all(struct guard_sock *gs, const void *buf, size_t n,
			 __nsec deadline)
{
	size_t off = 0;

	while (off < n) {
		ssize_t r = tcp_io(gs, 1, (__u8 *)buf + off, n - off);
		int w;

		if (r > 0) {
			off += (size_t)r;
			continue;
		}
		if (r == 0)
			return -EPIPE;
		if (r != -EAGAIN)
			return (int)r;
		w = raw_wait(gs, EPOLLOUT | EPOLLERR | EPOLLHUP, deadline);
		if (w < 0)
			return w;
	}
	return 0;
}

/* Exactly ONE TLS record (5-byte header + body) from the TCP peer. */
static long read_one_record(struct guard_sock *gs, __u8 *buf, __nsec deadline)
{
	size_t len;
	ssize_t r;
	int rc;

	r = tcp_read_some(gs, buf, 1, deadline);
	if (r <= 0)
		return r; /* 0 = EOF before any byte */
	rc = tcp_read_exact(gs, buf + 1, REC_HDR - 1, deadline);
	if (rc)
		return rc;
	len = ((size_t)buf[3] << 8) | buf[4];
	if (len > REC_MAX_CT)
		return -EPROTO;
	rc = len ? tcp_read_exact(gs, buf + REC_HDR, len, deadline) : 0;
	if (rc)
		return rc;
	return (long)(REC_HDR + len);
}

/* ---- vsock I/O (Spike B) -------------------------------------------------- */

static ssize_t vs_read(const struct uk_file *f, void *buf, size_t n,
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

static int vs_read_exact(const struct uk_file *f, void *buf, size_t n,
			 __nsec deadline)
{
	size_t got = 0;

	while (got < n) {
		ssize_t r = vs_read(f, (__u8 *)buf + got, n - got, deadline);

		if (r < 0)
			return (int)r;
		if (r == 0)
			return -ECONNRESET;
		got += (size_t)r;
	}
	return 0;
}

static int vs_write_all(const struct uk_file *f, const void *buf, size_t n,
			__nsec deadline)
{
	size_t off = 0;

	while (off < n) {
		struct iovec iov = { .iov_base = (__u8 *)buf + off,
				     .iov_len = n - off };
		ssize_t r = posix_socket_write(f, &iov, 1);

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

static int frame_send(const struct uk_file *vs, unsigned int id, __u8 t,
		      const void *p, __u32 len, __nsec deadline)
{
	__u8 hdr[5] = { t, (__u8)(len >> 24), (__u8)(len >> 16),
			(__u8)(len >> 8), (__u8)len };
	int r = vs_write_all(vs, hdr, sizeof(hdr), deadline);

	if (!r && len)
		r = vs_write_all(vs, p, len, deadline);
	printf("MTLSGUARD: [c%u] vsock tx %s payload=%u wire=%u%s t=%s\n", id,
	       tname(t), (unsigned int)len, (unsigned int)len + 5,
	       r ? " FAILED" : "", MONO());
	return r;
}

static long frame_recv(const struct uk_file *vs, __u8 *t, __u8 *buf,
		       size_t cap, __nsec deadline)
{
	__u8 hdr[5];
	__u32 len;
	int r = vs_read_exact(vs, hdr, sizeof(hdr), deadline);

	if (r)
		return r;
	len = ((__u32)hdr[1] << 24) | ((__u32)hdr[2] << 16) |
	      ((__u32)hdr[3] << 8) | hdr[4];
	if (len > cap)
		return -EMSGSIZE;
	r = len ? vs_read_exact(vs, buf, len, deadline) : 0;
	if (r)
		return r;
	*t = hdr[0];
	return (long)len;
}

/* ---- record layer: TX ----------------------------------------------------- */

static void make_nonce(__u8 out[12], const __u8 iv[12], __u64 seq)
{
	int i;

	memcpy(out, iv, 12);
	for (i = 0; i < 8; i++)
		out[11 - i] ^= (__u8)(seq >> (8 * i));
}

static void tx_compact(struct guard_sock *gs)
{
	if (!gs->toff)
		return;
	memmove(gs->tbuf, gs->tbuf + gs->toff, gs->tlen - gs->toff);
	gs->tlen -= gs->toff;
	gs->toff = 0;
}

/* Encrypt n bytes gathered from iov as ONE record under the current TX keys
 * and append it to tbuf. Caller holds gs->lock. */
static int tx_record_locked(struct guard_sock *gs, const struct iovec *iov,
			    size_t iovcnt, size_t n, __u8 inner,
			    const char *what)
{
	size_t ctlen = n + 1 + REC_TAG, off = 0, i;
	__u8 nonce[12], *h;
	int r;

	if (TBUF_SIZE - gs->tlen < REC_HDR + ctlen)
		tx_compact(gs);
	if (TBUF_SIZE - gs->tlen < REC_HDR + ctlen)
		return -ENOBUFS;
	h = gs->tbuf + gs->tlen;
	for (i = 0; i < iovcnt && off < n; i++) {
		size_t take = MIN(iov[i].iov_len, n - off);

		memcpy(h + REC_HDR + off, iov[i].iov_base, take);
		off += take;
	}
	h[REC_HDR + n] = inner;
	h[0] = CT_APPDATA;
	h[1] = 0x03;
	h[2] = 0x03;
	h[3] = (__u8)(ctlen >> 8);
	h[4] = (__u8)ctlen;
	make_nonce(nonce, gs->tx.iv, gs->tx.seq);
	r = mbedtls_gcm_crypt_and_tag(&gs->tx.gcm, MBEDTLS_GCM_ENCRYPT, n + 1,
				      nonce, sizeof(nonce), h, REC_HDR,
				      h + REC_HDR, h + REC_HDR, REC_TAG,
				      h + REC_HDR + n + 1);
	if (r) {
		printf("MTLSGUARD: [c%u] GCM encrypt failed (%d)\n", gs->id, r);
		return -EIO;
	}
	gs->tlen += REC_HDR + ctlen;
	printf("MTLSGUARD: [c%u] tx record gen=%u seq=%llu %s plaintext=%lu wire=%lu (outer 0x17, inner 0x%02x) t=%s\n",
	       gs->id, gs->tx.gen, (unsigned long long)gs->tx.seq, what,
	       (unsigned long)n, (unsigned long)(REC_HDR + ctlen), inner,
	       MONO());
	gs->tx.seq++;
	return 0;
}

static int tx_control_locked(struct guard_sock *gs, const __u8 *p, size_t n,
			     __u8 inner, const char *what)
{
	struct iovec iov = { .iov_base = (void *)p, .iov_len = n };

	return tx_record_locked(gs, &iov, 1, n, inner, what);
}

/* Write queued ciphertext to lwIP. Returns 1 if some is left and lwIP is
 * not writable (the caller should wait for raw EPOLLOUT). */
static int tx_flush_locked(struct guard_sock *gs)
{
	while (gs->toff < gs->tlen && !gs->tx_err) {
		struct iovec iov = { .iov_base = gs->tbuf + gs->toff,
				     .iov_len = gs->tlen - gs->toff };
		ssize_t r = gs->orig->ops->write(gs->sock, &iov, 1);

		if (r > 0) {
			gs->toff += (size_t)r;
			continue;
		}
		if (r == -EAGAIN)
			return 1;
		gs->tx_err = r ? (int)r : -EPIPE;
		printf("MTLSGUARD: [c%u] lwIP write failed (%d): stop flushing\n",
		       gs->id, gs->tx_err);
	}
	if (gs->toff == gs->tlen)
		gs->toff = gs->tlen = 0;
	if (!gs->tlen && gs->shut_req && !gs->shut_done) {
		gs->shut_done = 1;
		(void)gs->orig->ops->shutdown(gs->sock, gs->shut_how);
		printf("MTLSGUARD: [c%u] TX drained -> lwIP shutdown(%s) t=%s\n",
		       gs->id, gs->shut_how == SHUT_WR ? "SHUT_WR" : "SHUT_RDWR",
		       MONO());
	}
	return 0;
}

/* Fail the connection closed: no more data in either direction, an alert to
 * the peer if we can still encrypt, then the TCP connection is shut down. */
static void fatal_locked(struct guard_sock *gs, int err, int alert,
			 const char *why)
{
	printf("MTLSGUARD: [c%u] FAIL CLOSED: %s -> read()/write() now return %d%s t=%s\n",
	       gs->id, why, err, alert >= 0 ? ", fatal alert sent" : "",
	       MONO());
	if (alert >= 0 && gs->keys_ready && !gs->tx_closed && !gs->tx_err) {
		__u8 a[2] = { 2, (__u8)alert };

		(void)tx_control_locked(gs, a, 2, CT_ALERT, "fatal alert");
	}
	if (!gs->rx_err)
		gs->rx_err = err;
	if (!gs->tx_closed)
		gs->tx_closed = err;
	gs->hslen = 0;
	gs->shut_req = 1;
	gs->shut_how = SHUT_RDWR;
}

/* ---- record layer: RX ----------------------------------------------------- */

static const char *hs_name(unsigned int t)
{
	switch (t) {
	case HS_NST: return "NewSessionTicket";
	case HS_KEYUPDATE: return "KeyUpdate";
	case 13: return "CertificateRequest";
	case 20: return "Finished";
	case 8: return "EncryptedExtensions";
	default: return "other";
	}
}

/* Append the plaintext of one inner-0x16 record and process every complete
 * handshake message; a partial message waits for the next record. */
static void rx_handshake_locked(struct guard_sock *gs, const __u8 *p, size_t n,
				__u64 seq)
{
	size_t off = 0;

	if (gs->hslen + n > HS_MAX) {
		fatal_locked(gs, -EPROTO, ALERT_UNEXPECTED_MESSAGE,
			     "post-handshake handshake data exceeds the reassembly buffer");
		return;
	}
	memcpy(gs->hsbuf + gs->hslen, p, n);
	gs->hslen += n;
	while (gs->hslen - off >= 4) {
		unsigned int t = gs->hsbuf[off];
		size_t mlen = ((size_t)gs->hsbuf[off + 1] << 16) |
			      ((size_t)gs->hsbuf[off + 2] << 8) |
			      gs->hsbuf[off + 3];
		const __u8 *body;

		if (mlen > HS_MAX - 4) {
			fatal_locked(gs, -EPROTO, ALERT_DECODE_ERROR,
				     "post-handshake message length too large");
			return;
		}
		if (gs->hslen - off < 4 + mlen)
			break; /* continues in the next record */
		body = gs->hsbuf + off + 4;
		off += 4 + mlen;
		switch (t) {
		case HS_NST:
			gs->nst++;
			printf("MTLSGUARD: [c%u] rx post-handshake NewSessionTicket #%lu (%lu B body) in record seq=%llu -> discarded (no resumption) t=%s\n",
			       gs->id, gs->nst, (unsigned long)mlen,
			       (unsigned long long)seq, MONO());
			break;
		case HS_KEYUPDATE: {
			unsigned int req;

			if (mlen != 1 || body[0] > 1) {
				fatal_locked(gs, -EPROTO, ALERT_DECODE_ERROR,
					     "malformed KeyUpdate");
				return;
			}
			if (off != gs->hslen) {
				fatal_locked(gs, -EPROTO,
					     ALERT_UNEXPECTED_MESSAGE,
					     "KeyUpdate not at a record boundary");
				return;
			}
			req = body[0];
			if (keys_update(&gs->rx)) {
				fatal_locked(gs, -EIO, -1, "rx key update failed");
				return;
			}
			gs->ku_rx++;
			printf("MTLSGUARD: [c%u] rx KeyUpdate(%s) in record seq=%llu -> rx traffic secret N+1 = HKDF-Expand-Label(secret_N, \"traffic upd\"), rx keys now generation %u, rx seq 0 t=%s\n",
			       gs->id, req ? "update_requested" :
			       "update_not_requested", (unsigned long long)seq,
			       gs->rx.gen, MONO());
			if (req) {
				static const __u8 ku[5] = { HS_KEYUPDATE, 0, 0,
							    1, 0 };

				if (tx_control_locked(gs, ku, sizeof(ku),
						      CT_HANDSHAKE,
						      "KeyUpdate(update_not_requested)") ||
				    keys_update(&gs->tx)) {
					fatal_locked(gs, -EIO, -1,
						     "tx key update failed");
					return;
				}
				gs->ku_tx++;
				printf("MTLSGUARD: [c%u] sent KeyUpdate(update_not_requested) under the old tx keys, then tx keys -> generation %u, tx seq 0 t=%s\n",
				       gs->id, gs->tx.gen, MONO());
			}
			break;
		}
		default: {
			char why[96];

			snprintf(why, sizeof(why),
				 "unexpected post-handshake handshake message type %u (%s)",
				 t, hs_name(t));
			fatal_locked(gs, -EPROTO, ALERT_UNEXPECTED_MESSAGE, why);
			return;
		}
		}
	}
	memmove(gs->hsbuf, gs->hsbuf + off, gs->hslen - off);
	gs->hslen -= off;
	if (gs->hslen)
		printf("MTLSGUARD: [c%u] rx handshake message continues in the next record (%lu B buffered)\n",
		       gs->id, (unsigned long)gs->hslen);
}

/* Decrypt the complete record in rbuf. Caller holds gs->lock. */
static void rx_record_locked(struct guard_sock *gs)
{
	size_t ctlen = ((size_t)gs->rbuf[3] << 8) | gs->rbuf[4];
	size_t n = ctlen - REC_TAG, i;
	__u64 seq = gs->rx.seq;
	__u8 nonce[12];
	__u8 inner;
	int r;

	make_nonce(nonce, gs->rx.iv, seq);
	r = mbedtls_gcm_auth_decrypt(&gs->rx.gcm, n, nonce, sizeof(nonce),
				     gs->rbuf, REC_HDR, gs->rbuf + REC_HDR + n,
				     REC_TAG, gs->rbuf + REC_HDR, gs->pbuf);
	gs->rlen = 0;
	if (r) {
		fatal_locked(gs, -EBADMSG, ALERT_BAD_RECORD_MAC,
			     "record authentication failed");
		return;
	}
	gs->rx.seq++;
	i = n;
	while (i > 0 && gs->pbuf[i - 1] == 0)
		i--;
	if (i == 0) {
		fatal_locked(gs, -EPROTO, ALERT_UNEXPECTED_MESSAGE,
			     "record without a content type");
		return;
	}
	inner = gs->pbuf[i - 1];
	printf("MTLSGUARD: [c%u] rx record gen=%u seq=%llu wire=%lu inner=0x%02x plaintext=%lu t=%s\n",
	       gs->id, gs->rx.gen, (unsigned long long)seq,
	       (unsigned long)(REC_HDR + ctlen), inner, (unsigned long)(i - 1),
	       MONO());
	switch (inner) {
	case CT_APPDATA:
		if (gs->hslen) {
			fatal_locked(gs, -EPROTO, ALERT_UNEXPECTED_MESSAGE,
				     "application data inside a fragmented handshake message");
			return;
		}
		if (i - 1 == 0)
			return; /* zero-length application data: nothing to read */
		gs->poff = 0;
		gs->plen = i - 1;
		gs->rx_app_records++;
		return;
	case CT_ALERT:
		if (i - 1 == 2 && gs->pbuf[1] == 0) {
			printf("MTLSGUARD: [c%u] rx close_notify -> EOF\n",
			       gs->id);
			gs->rx_eof = 1;
			return;
		}
		printf("MTLSGUARD: [c%u] rx alert level=%u desc=%u\n", gs->id,
		       gs->pbuf[0], i > 2 ? gs->pbuf[1] : 0);
		fatal_locked(gs, -ECONNRESET, -1, "peer sent a fatal alert");
		return;
	case CT_HANDSHAKE:
		rx_handshake_locked(gs, gs->pbuf, i - 1, seq);
		return;
	default:
		fatal_locked(gs, -EPROTO, ALERT_UNEXPECTED_MESSAGE,
			     "unexpected inner content type");
		return;
	}
}

/* Read and decrypt until one application record's plaintext is buffered, the
 * stream ends, or lwIP has no more bytes. Returns 1 when waiting for lwIP. */
static int rx_advance_locked(struct guard_sock *gs)
{
	while (gs->poff >= gs->plen && !gs->rx_eof && !gs->rx_err) {
		size_t need = REC_HDR;
		struct iovec iov;
		ssize_t r;

		if (gs->rlen >= REC_HDR) {
			size_t ctlen = ((size_t)gs->rbuf[3] << 8) | gs->rbuf[4];

			if (gs->rbuf[0] != CT_APPDATA || gs->rbuf[1] != 0x03 ||
			    gs->rbuf[2] != 0x03 || ctlen < REC_TAG + 1 ||
			    ctlen > REC_MAX_CT) {
				gs->rlen = 0;
				fatal_locked(gs, -EPROTO,
					     ALERT_UNEXPECTED_MESSAGE,
					     "bad record header");
				break;
			}
			need = REC_HDR + ctlen;
			if (gs->rlen == need) {
				rx_record_locked(gs);
				continue;
			}
		}
		iov.iov_base = gs->rbuf + gs->rlen;
		iov.iov_len = need - gs->rlen;
		r = gs->orig->ops->read(gs->sock, &iov, 1);
		if (r == -EAGAIN)
			return 1;
		if (r < 0) {
			printf("MTLSGUARD: [c%u] lwIP read failed (%ld)\n",
			       gs->id, (long)r);
			gs->rx_err = (int)r;
			if (!gs->tx_closed)
				gs->tx_closed = (int)r;
			break;
		}
		if (r == 0) {
			if (gs->rlen || gs->hslen) {
				printf("MTLSGUARD: [c%u] TCP EOF mid-record -> fail closed\n",
				       gs->id);
				gs->rx_err = -EPROTO;
			} else {
				printf("MTLSGUARD: [c%u] TCP EOF without close_notify -> EOF\n",
				       gs->id);
				gs->rx_eof = 1;
			}
			break;
		}
		gs->rlen += (size_t)r;
	}
	return 0;
}

/* ---- guard driver ops (application side) --------------------------------- */

#define GS(f) ((struct guard_sock *)posix_sock_get_driver(f)->private)

static size_t copy_out(struct guard_sock *gs, const struct iovec *iov,
		       size_t iovcnt, int peek)
{
	size_t done = 0, off = gs->poff, i;

	for (i = 0; i < iovcnt && off < gs->plen; i++) {
		size_t take = MIN(iov[i].iov_len, gs->plen - off);

		memcpy(iov[i].iov_base, gs->pbuf + off, take);
		off += take;
		done += take;
	}
	if (!peek)
		gs->poff = off;
	return done;
}

/* The application's read path copies decrypted plaintext only; decryption is
 * the RX pump's job (so EPOLLIN only ever means "plaintext is ready"). */
static ssize_t guard_recvv(const struct uk_file *f, const struct iovec *iov,
			   size_t iovcnt, int peek)
{
	struct guard_sock *gs = GS(f);
	int wake = 0;
	ssize_t r;

	uk_mutex_lock(&gs->lock);
	if (gs->state == GS_HANDSHAKING) {
		gs->hs_eagain_reads++;
		r = -EAGAIN;
	} else if (gs->state == GS_FAILED) {
		r = gs->err;
	} else if (gs->poff < gs->plen) {
		r = (ssize_t)copy_out(gs, iov, iovcnt, peek);
		if (gs->poff == gs->plen) {
			gs->poff = gs->plen = 0;
			wake = 1; /* the pump may decrypt the next record */
		}
	} else if (gs->rx_err) {
		r = gs->rx_err;
	} else if (gs->rx_eof) {
		r = 0;
	} else {
		r = -EAGAIN;
	}
	uk_mutex_unlock(&gs->lock);
	publish(gs);
	if (wake)
		kick(gs);
	return r;
}

static ssize_t guard_sendv(const struct uk_file *f, const struct iovec *iov,
			   size_t iovcnt)
{
	struct guard_sock *gs = GS(f);
	size_t total = 0, n, i;
	int pending = 0;
	ssize_t r;
	long room;

	for (i = 0; i < iovcnt; i++)
		total += iov[i].iov_len;
	uk_mutex_lock(&gs->lock);
	if (gs->state == GS_HANDSHAKING) {
		if (!gs->hs_eagain_writes++)
			printf("MTLSGUARD: [c%u] write() of %lu B before the record layer is installed -> EAGAIN, 0 bytes to lwIP t=%s\n",
			       gs->id, (unsigned long)total, MONO());
		r = -EAGAIN;
		goto out;
	}
	if (gs->state == GS_FAILED) {
		r = gs->err;
		goto out;
	}
	if (gs->tx_closed || gs->tx_err) {
		r = gs->tx_err ? gs->tx_err : gs->tx_closed;
		goto out;
	}
	if (total == 0) {
		r = 0;
		goto out;
	}
	room = tx_room(gs) - REC_OVERHEAD;
	if (room <= 0) {
		r = -EAGAIN;
		goto out;
	}
	n = MIN(MIN(total, (size_t)REC_MAX_PLAIN), (size_t)room);
	r = tx_record_locked(gs, iov, iovcnt, n, CT_APPDATA, "appdata");
	if (r)
		goto out;
	gs->tx_app_records++;
	pending = tx_flush_locked(gs);
	r = (ssize_t)n;
out:
	uk_mutex_unlock(&gs->lock);
	publish(gs);
	if (pending)
		kick(gs);
	return r;
}

static ssize_t g_read(posix_sock *f, const struct iovec *iov, size_t n)
{
	return guard_recvv(f, iov, n, 0);
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

	if (from && fromlen)
		*fromlen = 0;
	return guard_recvv(f, &iov, 1, !!(flags & MSG_PEEK));
}

static ssize_t g_recvmsg(posix_sock *f, struct msghdr *msg, int flags)
{
	msg->msg_controllen = 0;
	msg->msg_flags = 0;
	msg->msg_namelen = 0;
	return guard_recvv(f, msg->msg_iov, msg->msg_iovlen,
			   !!(flags & MSG_PEEK));
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

static void queue_close_notify_locked(struct guard_sock *gs, int err)
{
	if (gs->state == GS_ESTABLISHED && !gs->tx_closed && !gs->tx_err) {
		static const __u8 cn[2] = { 1, 0 };

		(void)tx_control_locked(gs, cn, 2, CT_ALERT, "close_notify");
	}
	if (!gs->tx_closed)
		gs->tx_closed = err;
}

static int g_shutdown(posix_sock *f, int how)
{
	struct guard_sock *gs = GS(f);
	int r = 0;

	uk_mutex_lock(&gs->lock);
	if (gs->state != GS_ESTABLISHED) {
		r = gs->orig->ops->shutdown(f, how);
	} else if (how == SHUT_RD) {
		r = gs->orig->ops->shutdown(f, how);
	} else {
		queue_close_notify_locked(gs, -EPIPE);
		gs->shut_req = 1;
		gs->shut_how = how;
		(void)tx_flush_locked(gs);
	}
	uk_mutex_unlock(&gs->lock);
	publish(gs);
	kick(gs);
	return r;
}

static int g_close(posix_sock *f)
{
	struct guard_sock *gs = GS(f);
	struct posix_socket_driver *orig = gs->orig;
	__nsec deadline;

	uk_mutex_lock(&gs->lock);
	printf("MTLSGUARD: [c%u] close: state=%s tx records=%lu rx records=%lu NewSessionTickets discarded=%lu KeyUpdates rx=%lu tx=%lu\n",
	       gs->id, gs->state == GS_ESTABLISHED ? "established" :
	       gs->state == GS_FAILED ? "failed" : "handshaking",
	       gs->tx_app_records, gs->rx_app_records, gs->nst, gs->ku_rx,
	       gs->ku_tx);
	queue_close_notify_locked(gs, -EPIPE);
	gs->closing = 1;
	uk_mutex_unlock(&gs->lock);
	kick(gs);
	if (gs->vs)
		(void)posix_socket_shutdown(gs->vs, SHUT_RDWR);
	/* Join: the guard thread flushes what is queued (bounded) and exits. */
	deadline = ukplat_monotonic_clock() + CLOSE_JOIN_NS;
	while (!gs->thread_done && ukplat_monotonic_clock() < deadline)
		uk_sched_thread_sleep(1000000);
	if (!gs->thread_done) {
		printf("MTLSGUARD: [c%u] guard thread did not exit; leaking its state\n",
		       gs->id);
		posix_sock_get_node(f)->driver = orig;
		return orig->ops->close(f);
	}
	/* Restore lwIP's driver before freeing: socket_release() frees the
	 * socket with node.driver->allocator after this returns. */
	posix_sock_get_node(f)->driver = orig;
	mbedtls_gcm_free(&gs->tx.gcm);
	mbedtls_gcm_free(&gs->rx.gcm);
	mbedtls_platform_zeroize(gs, sizeof(*gs));
	uk_free(orig->allocator, gs);
	return orig->ops->close(f);
}

static int g_getsockopt(posix_sock *f, int lv, int o, void *restrict v,
			socklen_t *restrict l)
{
	struct guard_sock *gs = GS(f);

	if (lv == SOL_SOCKET && o == SO_ERROR && v && l && *l >= sizeof(int) &&
	    gs->state != GS_ESTABLISHED) {
		int e = 0;

		if (gs->state == GS_FAILED && !gs->so_error_reported) {
			e = -gs->err;
			gs->so_error_reported = 1;
		}
		*(int *)v = e;
		*l = sizeof(int);
		return 0;
	}
	return gs->orig->ops->getsockopt(f, lv, o, v, l);
}

static int g_ioctl(posix_sock *f, int req, void *argp)
{
	struct guard_sock *gs = GS(f);

	if (req == FIONREAD && argp) {
		*(int *)argp = gs->state == GS_ESTABLISHED ?
			       (int)(gs->plen - gs->poff) : 0;
		return 0;
	}
	return gs->orig->ops->ioctl(f, req, argp);
}

static int g_connect(posix_sock *f, const struct sockaddr *a __unused,
		     socklen_t l __unused)
{
	struct guard_sock *gs = GS(f);

	return gs->state == GS_HANDSHAKING ? -EALREADY :
	       gs->state == GS_FAILED ? gs->err : -EISCONN;
}

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
static int g_setsockopt(posix_sock *f, int lv, int o, const void *v,
			socklen_t l)
{ return GS(f)->orig->ops->setsockopt(f, lv, o, v, l); }
static int g_listen(posix_sock *f __unused, int b __unused)
{ return -EOPNOTSUPP; }
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

/* ---- handshake relay (guard thread) --------------------------------------- */

static __u64 be64(const __u8 *p)
{
	__u64 v = 0;
	int i;

	for (i = 0; i < 8; i++)
		v = (v << 8) | p[i];
	return v;
}

/* SECRETS v2: suite(2) | tx dir | rx dir | idlen(2) | id, where a dir is
 * seq(8) slen(1)=32 secret(32) klen(1)=16 key(16) iv(12) = 70 bytes. */
#define SEC_DIR 70

static int install_secrets(struct guard_sock *gs, const __u8 *p, long len)
{
	const __u8 *d[2] = { p + 2, p + 2 + SEC_DIR };
	struct dir_keys *k[2] = { &gs->tx, &gs->rx };
	int i, r;
	size_t idlen;

	if (len < 2 + 2 * SEC_DIR + 2 || p[0] != 0x13 || p[1] != 0x01) {
		printf("MTLSGUARD: [c%u] malformed SECRETS (len=%ld)\n", gs->id,
		       len);
		return -EPROTO;
	}
	for (i = 0; i < 2; i++) {
		__u8 key[16], iv[12];

		if (d[i][8] != 32 || d[i][9 + 32] != 16) {
			printf("MTLSGUARD: [c%u] SECRETS: unexpected secret/key length\n",
			       gs->id);
			return -EPROTO;
		}
		r = hkdf_expand_label(d[i] + 9, "key", key, 16);
		if (!r)
			r = hkdf_expand_label(d[i] + 9, "iv", iv, 12);
		if (r || memcmp(key, d[i] + 42, 16) || memcmp(iv, d[i] + 58, 12)) {
			printf("MTLSGUARD: [c%u] SECRETS: guest HKDF-Expand-Label(%s traffic secret) does NOT match rustls key/iv -> fail closed\n",
			       gs->id, i ? "rx" : "tx");
			mbedtls_platform_zeroize(key, sizeof(key));
			return -EPROTO;
		}
		mbedtls_platform_zeroize(key, sizeof(key));
	}
	idlen = ((size_t)p[2 + 2 * SEC_DIR] << 8) | p[2 + 2 * SEC_DIR + 1];
	if (idlen >= sizeof(gs->peer) ||
	    (long)(2 + 2 * SEC_DIR + 2 + idlen) > len)
		idlen = 0;
	uk_mutex_lock(&gs->lock);
	for (i = 0; i < 2; i++) {
		mbedtls_gcm_init(&k[i]->gcm);
		r = keys_set(k[i], d[i] + 9);
		k[i]->seq = be64(d[i]);
		k[i]->gen = 0;
		if (r) {
			uk_mutex_unlock(&gs->lock);
			printf("MTLSGUARD: [c%u] key setup failed (%d)\n",
			       gs->id, r);
			return -EIO;
		}
	}
	memcpy(gs->peer, p + 2 + 2 * SEC_DIR + 2, idlen);
	gs->peer[idlen] = '\0';
	gs->keys_ready = 1;
	gs->t_installed = ukplat_monotonic_clock();
	gs->state = GS_ESTABLISHED;
	uk_mutex_unlock(&gs->lock);
	printf("MTLSGUARD: [c%u] record layer installed: TLS_AES_128_GCM_SHA256 tx_seq=%llu rx_seq=%llu; key/iv derived in the guest by HKDF-Expand-Label(SHA-256) from the traffic secrets and equal to rustls' key/iv (tx and rx); peer identity (from host) %s; app write() EAGAINs during the handshake=%lu, read() EAGAINs=%lu t=%s\n",
	       gs->id, (unsigned long long)gs->tx.seq,
	       (unsigned long long)gs->rx.seq, gs->peer, gs->hs_eagain_writes,
	       gs->hs_eagain_reads, MONO());
	return 0;
}

static int relay_handshake(struct guard_sock *gs, __nsec deadline)
{
	struct sockaddr_vm svm;
	struct uk_file *vs;
	unsigned int n_to = 0, n_need = 0, b_to = 0, b_from = 0;
	__u8 *fbuf;
	int r, ret;

	fbuf = uk_malloc(gs->orig->allocator, MAX_FRAME);
	if (!fbuf)
		return -ENOMEM;
	vs = uk_socket_create(AF_VSOCK, SOCK_STREAM, 0);
	if (PTRISERR(vs)) {
		printf("MTLSGUARD: [c%u] vsock socket failed (%d)\n", gs->id,
		       PTR2ERR(vs));
		uk_free(gs->orig->allocator, fbuf);
		return -ECONNABORTED;
	}
	gs->vs = vs;
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
		       gs->id, (unsigned int)agent_port, r, MONO());
		ret = -ECONNABORTED;
		goto out;
	}
	printf("MTLSGUARD: [c%u] vsock connected to host relay (cid 2 port %u) t=%s\n",
	       gs->id, (unsigned int)agent_port, MONO());
	if (frame_send(vs, gs->id, gs->open_type, gs->open, gs->open_len,
		       deadline)) {
		ret = -ECONNABORTED;
		goto out;
	}
	for (;;) {
		__u8 t = 0;
		long len = frame_recv(vs, &t, fbuf, MAX_FRAME, deadline);

		if (len < 0) {
			printf("MTLSGUARD: [c%u] vsock recv failed (%ld) -> fail closed\n",
			       gs->id, len);
			ret = gs->closing ? -ECANCELED : -ECONNABORTED;
			goto out;
		}
		printf("MTLSGUARD: [c%u] vsock rx %s payload=%ld wire=%ld t=%s\n",
		       gs->id, tname(t), len, len + 5, MONO());
		switch (t) {
		case T_TO_PEER:
			n_to++;
			b_to += (unsigned int)len;
			r = tcp_write_all(gs, fbuf, (size_t)len, deadline);
			printf("MTLSGUARD: [c%u] tcp tx %ld handshake bytes to peer%s\n",
			       gs->id, len, r ? " FAILED" : "");
			if (r) {
				ret = -ECONNABORTED;
				goto out;
			}
			break;
		case T_NEED_PEER: {
			long got;

			n_need++;
			got = read_one_record(gs, fbuf, deadline);
			if (got < 0) {
				printf("MTLSGUARD: [c%u] tcp rx failed (%ld)\n",
				       gs->id, got);
				ret = -ECONNABORTED;
				goto out;
			}
			b_from += (unsigned int)got;
			printf("MTLSGUARD: [c%u] tcp rx one handshake record from peer: %ld bytes (type 0x%02x)\n",
			       gs->id, got, got ? fbuf[0] : 0);
			if (frame_send(vs, gs->id, T_FROM_PEER, fbuf,
				       (__u32)got, deadline)) {
				ret = -ECONNABORTED;
				goto out;
			}
			break;
		}
		case T_SECRETS:
			ret = install_secrets(gs, fbuf, len);
			mbedtls_platform_zeroize(fbuf, (size_t)len);
			goto out;
		case T_DENY:
			fbuf[len < MAX_FRAME ? len : MAX_FRAME - 1] = 0;
			printf("MTLSGUARD: [c%u] host DENY: %s\n", gs->id, fbuf);
			ret = gs->role == ROLE_ACCEPT ? -ECONNABORTED : -EACCES;
			goto out;
		case T_ERROR:
			fbuf[len < MAX_FRAME ? len : MAX_FRAME - 1] = 0;
			printf("MTLSGUARD: [c%u] host ERROR: %s\n", gs->id,
			       fbuf);
			ret = -ECONNABORTED;
			goto out;
		default:
			printf("MTLSGUARD: [c%u] unknown frame type 0x%02x -> fail closed\n",
			       gs->id, t);
			ret = -EPROTO;
			goto out;
		}
	}
out:
	printf("MTLSGUARD: [c%u] vsock summary: rx TO_PEER x%u (%u B), NEED_PEER x%u; tx FROM_PEER %u B; closing vsock (agent leaves the data path) t=%s\n",
	       gs->id, n_to, b_to, n_need, b_from, MONO());
	gs->vs = NULL;
	uk_file_release(vs);
	uk_free(gs->orig->allocator, fbuf);
	return ret;
}

static void handshake_failed(struct guard_sock *gs, int err)
{
	uk_mutex_lock(&gs->lock);
	gs->state = GS_FAILED;
	gs->err = err;
	(void)gs->orig->ops->shutdown(gs->sock, SHUT_RDWR);
	uk_mutex_unlock(&gs->lock);
	printf("MTLSGUARD: [c%u] %s %s: FAIL closed (%d), TCP shut down, no application data sent (after %llu us) t=%s\n",
	       gs->id, gs->role == ROLE_ACCEPT ? "accept" : "connect", gs->desc,
	       err,
	       (unsigned long long)(ukplat_monotonic_clock() - gs->t_start) / 1000ull,
	       MONO());
	publish(gs);
}

/* ---- guard thread: handshake, then RX pump -------------------------------- */

static void pump(struct guard_sock *gs)
{
	unsigned int spins = 0;

	for (;;) {
		unsigned int mask = GS_EV_KICK;
		int need_in, need_out;

		uk_pollq_clear(&gs->rawq, GS_EV_KICK);
		if (gs->closing)
			return;
		uk_mutex_lock(&gs->lock);
		need_in = rx_advance_locked(gs);
		need_out = tx_flush_locked(gs);
		uk_mutex_unlock(&gs->lock);
		publish(gs);
		if (need_in)
			mask |= EPOLLIN | EPOLLRDHUP | EPOLLERR | EPOLLHUP;
		if (need_out)
			mask |= EPOLLOUT | EPOLLERR | EPOLLHUP;
		/* Spin guard: lwIP reports readiness we cannot consume. */
		if ((mask & ~GS_EV_KICK) && (gs->lwip_ev & mask & ~GS_EV_KICK)) {
			if (++spins == 1000)
				printf("MTLSGUARD: [c%u] pump: lwIP readiness 0x%x persists without progress; backing off\n",
				       gs->id, gs->lwip_ev);
			if (spins >= 1000)
				uk_sched_thread_sleep(1000000);
		} else {
			spins = 0;
		}
		(void)uk_pollq_poll_until(&gs->rawq, mask, 0, NULL);
	}
}

static void flush_on_close(struct guard_sock *gs)
{
	__nsec deadline = ukplat_monotonic_clock() + CLOSE_FLUSH_NS;

	for (;;) {
		int left;

		uk_mutex_lock(&gs->lock);
		(void)tx_flush_locked(gs);
		left = gs->toff < gs->tlen && !gs->tx_err;
		uk_mutex_unlock(&gs->lock);
		if (!left)
			return;
		if (!uk_pollq_poll_until(&gs->rawq, EPOLLOUT | EPOLLERR,
					 deadline, NULL))
			return;
	}
}

/* Unikraft thread entry functions are __noreturn: ukarch_ctx_init_entry1()
 * leaves a 0 return address, so the thread must end in uk_sched_thread_exit(). */
static void __noreturn guard_thread(void *arg)
{
	struct guard_sock *gs = arg;
	__nsec deadline = gs->t_start + HANDSHAKE_TIMEOUT_NS;
	int r = 0;

	if (gs->tcp_connecting) {
		/* Non-blocking connect: wait for lwIP's own EPOLLOUT (TCP up). */
		r = raw_wait(gs, EPOLLOUT | EPOLLERR | EPOLLHUP, deadline);
		while (!r && !(gs->lwip_ev & (EPOLLOUT | EPOLLERR | EPOLLHUP)))
			r = raw_wait(gs, EPOLLOUT | EPOLLERR | EPOLLHUP,
				     deadline);
		if (!r) {
			int soerr = 0;
			socklen_t sl = sizeof(soerr);

			uk_mutex_lock(&gs->lock);
			gs->orig->ops->getsockopt(gs->sock, SOL_SOCKET,
						  SO_ERROR, &soerr, &sl);
			uk_mutex_unlock(&gs->lock);
			if (soerr)
				r = soerr < 0 ? soerr : -soerr;
		}
		if (!r) {
			gs->t_tcp_up = ukplat_monotonic_clock();
			printf("MTLSGUARD: [c%u] TCP connected after %llu us (lwIP raw EPOLLOUT at t=%s); the app still sees no EPOLLOUT -> relay the handshake\n",
			       gs->id,
			       (unsigned long long)(gs->t_tcp_up - gs->t_start) / 1000ull,
			       fmt_ns((char[32]){0}, 32, gs->t_raw_out_first));
		}
	}
	if (!r)
		r = relay_handshake(gs, deadline);
	if (r) {
		handshake_failed(gs, r);
	} else {
		publish(gs); /* EPOLLOUT rises here, after the install log line */
		printf("MTLSGUARD: [c%u] %s %s: handshake done after %llu us; RX pump running t=%s\n",
		       gs->id, gs->role == ROLE_ACCEPT ? "accept" : "connect",
		       gs->desc,
		       (unsigned long long)(ukplat_monotonic_clock() - gs->t_start) / 1000ull,
		       MONO());
	}
	uk_pollq_set(&gs->rawq, GS_EV_HSDONE);
	if (!r)
		pump(gs);
	flush_on_close(gs);
	gs->thread_done = 1; /* last touch of gs: g_close may free it now */
	uk_sched_thread_exit();
}

/* ---- install at connect()/accept() ----------------------------------------- */

static struct guard_sock *guard_new(const struct uk_file *sock, int role,
				    const char *desc, __u8 open_type,
				    const __u8 *open, __u32 open_len)
{
	struct posix_socket_driver *orig = posix_sock_get_driver(sock);
	struct guard_sock *gs = uk_calloc(orig->allocator, 1, sizeof(*gs));

	if (!gs)
		return NULL;
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
	gs->sock = sock;
	gs->role = role;
	gs->id = ++conn_seq;
	snprintf(gs->desc, sizeof(gs->desc), "%s", desc);
	gs->open_type = open_type;
	memcpy(gs->open, open, open_len);
	gs->open_len = open_len;
	gs->state = GS_HANDSHAKING;
	gs->t_start = ukplat_monotonic_clock();
	uk_mutex_init(&gs->lock);
	gs->rawq = UK_POLLQ_MANAGED_INIT_VALUE(gs->rawq);
	return gs;
}

/* Swap the driver, re-seed readiness through lwIP (which now routes it via
 * our hook), and start the guard thread. */
static int guard_start(struct guard_sock *gs)
{
	struct uk_thread *t;

	posix_sock_get_node(gs->sock)->driver = &gs->drv;
	gs->orig->ops->poll_setup(gs->sock);
	printf("MTLSGUARD: [c%u] guard driver installed on %s: lwIP raw readiness 0x%x, app readiness 0x%x (withheld until the record layer is installed) t=%s\n",
	       gs->id, gs->desc, gs->lwip_ev, app_events(gs), MONO());
	t = uk_sched_thread_create(uk_sched_current(), guard_thread, gs,
				   "mtlsguard");
	if (!t) {
		gs->thread_done = 1;
		handshake_failed(gs, -ENOMEM);
		uk_pollq_set(&gs->rawq, GS_EV_HSDONE);
		return -ENOMEM;
	}
	return 0;
}

static int hold_until_done(struct guard_sock *gs)
{
	(void)uk_pollq_poll_until(&gs->rawq, GS_EV_HSDONE, 0, NULL);
	return gs->state == GS_ESTABLISHED ? 0 : gs->err;
}

int uk_socket_connect_hook(const struct uk_file *sock,
			   const struct sockaddr *addr, socklen_t addr_len,
			   int blocking, int ret)
{
	const struct sockaddr_in *sin = (const struct sockaddr_in *)addr;
	socklen_t tl = sizeof(int);
	struct guard_sock *gs;
	__u8 open[7];
	char dst[24];
	int type = 0, r;

	self_test_once();
	if (addr->sa_family != AF_INET || addr_len < sizeof(*sin))
		return ret;
	if (posix_socket_getsockopt(sock, SOL_SOCKET, SO_TYPE, &type, &tl) ||
	    type != SOCK_STREAM)
		return ret;
	fmt_dst(dst, sizeof(dst), sin);
	if (!is_mesh(mesh, sin)) {
		printf("MTLSGUARD: connect %s: not a mesh destination -> pass-through, driver untouched t=%s\n",
		       dst, MONO());
		return ret;
	}
	open[0] = 4;
	memcpy(open + 1, &sin->sin_addr.s_addr, 4);
	memcpy(open + 5, &sin->sin_port, 2);
	gs = guard_new(sock, ROLE_CONNECT, dst, T_OPEN, open, sizeof(open));
	if (!gs) {
		(void)posix_socket_shutdown(sock, SHUT_RDWR);
		return -ENOMEM;
	}
	gs->blocking = blocking;
	gs->tcp_connecting = (ret == -EINPROGRESS);
	printf("MTLSGUARD: [c%u] connect %s: mesh destination, %s connect (lwIP connect returned %d) t=%s\n",
	       gs->id, dst, blocking ? "BLOCKING" : "NON-BLOCKING", ret, MONO());
	r = guard_start(gs);
	if (r)
		return r;
	if (!blocking) {
		printf("MTLSGUARD: [c%u] connect %s: returning EINPROGRESS now; the guard thread drives TCP + handshake t=%s\n",
		       gs->id, dst, MONO());
		return -EINPROGRESS;
	}
	r = hold_until_done(gs);
	printf("MTLSGUARD: [c%u] connect %s: blocking hold released: connect() = %d after %llu us t=%s\n",
	       gs->id, dst, r,
	       (unsigned long long)(ukplat_monotonic_clock() - gs->t_start) / 1000ull,
	       MONO());
	return r;
}

int uk_socket_accept_hook(const struct uk_file *listener __unused,
			  const struct uk_file *sock, int blocking, int flags)
{
	struct sockaddr_in loc, rem;
	socklen_t ll = sizeof(loc), rl = sizeof(rem);
	char lstr[24], rstr[24], desc[64];
	struct guard_sock *gs;
	__u8 acc[13];
	int r;

	self_test_once();
	memset(&loc, 0, sizeof(loc));
	memset(&rem, 0, sizeof(rem));
	if (posix_socket_getsockname(sock, (struct sockaddr *)&loc, &ll) ||
	    loc.sin_family != AF_INET ||
	    posix_socket_getpeername(sock, (struct sockaddr *)&rem, &rl))
		return 0;
	fmt_dst(lstr, sizeof(lstr), &loc);
	fmt_dst(rstr, sizeof(rstr), &rem);
	if (!is_mesh(mesh_in, &loc)) {
		printf("MTLSGUARD: accept %s <- %s: not a mesh listener -> pass-through t=%s\n",
		       lstr, rstr, MONO());
		return 0;
	}
	snprintf(desc, sizeof(desc), "%s <- %s", lstr, rstr);
	acc[0] = 4;
	memcpy(acc + 1, &loc.sin_addr.s_addr, 4);
	memcpy(acc + 5, &loc.sin_port, 2);
	memcpy(acc + 7, &rem.sin_addr.s_addr, 4);
	memcpy(acc + 11, &rem.sin_port, 2);
	gs = guard_new(sock, ROLE_ACCEPT, desc, T_ACCEPT, acc, sizeof(acc));
	if (!gs)
		return -ENOMEM;
	gs->blocking = blocking;
	printf("MTLSGUARD: [c%u] accept %s: mesh listener, %s accept (new fd %s) t=%s\n",
	       gs->id, desc, blocking ? "BLOCKING" : "NON-BLOCKING",
	       (flags & SOCK_NONBLOCK) ? "non-blocking" : "blocking", MONO());
	r = guard_start(gs);
	if (r)
		return r;
	if (!blocking) {
		printf("MTLSGUARD: [c%u] accept %s: returning the fd now; handshake continues in the guard thread (app readiness withheld) t=%s\n",
		       gs->id, desc, MONO());
		return 0;
	}
	r = hold_until_done(gs);
	printf("MTLSGUARD: [c%u] accept %s: blocking hold released: %s after %llu us t=%s\n",
	       gs->id, desc, r ? "accept() fails" : "accept() gets the socket",
	       (unsigned long long)(ukplat_monotonic_clock() - gs->t_start) / 1000ull,
	       MONO());
	return r;
}
