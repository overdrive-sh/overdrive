// SPDX-License-Identifier: GPL-2.0
/*
 * Spike C re-run (Overdrive GH #303, increment-h) FULL MODULE. THROWAWAY.
 *
 * Camblet-style in-guest transparent mTLS as an out-of-tree LKM on a STOCK,
 * UNMODIFIED Linux kernel. Hooks tcp_prot connect/sendmsg/recvmsg.
 *
 *   tcp_prot.connect is invoked by inet_stream_connect WITH lock_sock(sk) HELD,
 *   so the hook there only does bookkeeping (record the mesh dst for sk) and
 *   returns -- it CANNOT do socket I/O (that re-locks sk and deadlocks). The
 *   HOLD + handshake run lazily in the first sendmsg/recvmsg, which the kernel
 *   calls WITHOUT the socket lock. There the module:
 *     1. relays the TLS 1.3 handshake over vsock to the host rustls relay that
 *        holds the SVID key and decides policy (OPEN/TO_PEER/NEED_PEER/
 *        FROM_PEER/SECRETS/DENY, one TLS record per NEED_PEER);
 *     2. installs the returned session keys into the kernel's kTLS ULP on the
 *        workload's OWN socket (tcp_set_ulp "tls" + TLS_TX/TLS_RX), then steps
 *        out: the kernel carries the record layer, agent out of the data path;
 *     3. forwards the app's first I/O to the now-kTLS socket.
 * No record layer, no crypto, no KeyUpdate handling in the module -- the kernel
 * does all of that. The SVID private key never crosses vsock. DENY fails the
 * first I/O with -EACCES and no application bytes reach the wire.
 */
#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/kprobes.h>
#include <linux/net.h>
#include <linux/socket.h>
#include <linux/in.h>
#include <linux/inet.h>
#include <linux/sockptr.h>
#include <linux/tcp.h>
#include <linux/slab.h>
#include <linux/uio.h>
#include <linux/delay.h>
#include <linux/minmax.h>
#include <linux/spinlock.h>
#include <net/sock.h>
#include <net/tcp.h>
#include <net/tls.h>
#include <uapi/linux/vm_sockets.h>

MODULE_LICENSE("GPL");
MODULE_AUTHOR("Overdrive spike");
MODULE_DESCRIPTION("GH#303 Spike C re-run in-guest transparent-mTLS LKM, production-faithful TLS (throwaway)");

#define PFX "IGKMH: "

static char *mesh;
module_param(mesh, charp, 0444);
static uint agent_port = 7100;
module_param(agent_port, uint, 0444);
static uint agent_cid = VMADDR_CID_HOST; /* 2 */
module_param(agent_cid, uint, 0444);

#define T_OPEN 0x01
#define T_TO_PEER 0x02
#define T_NEED_PEER 0x03
#define T_FROM_PEER 0x04
#define T_SECRETS 0x05
#define T_DENY 0x06
#define T_ERROR 0x07
#define MAX_FRAME (64 * 1024)
#define REC_HDR 5
#define REC_MAX_CT (16384 + 256)

static struct proto *tcp_prot_p;
static int (*p_tcp_set_ulp)(struct sock *sk, const char *name);
static int (*orig_connect)(struct sock *sk, struct sockaddr_unsized *uaddr, int addr_len);
static int (*orig_sendmsg)(struct sock *sk, struct msghdr *msg, size_t size);
static int (*orig_recvmsg)(struct sock *sk, struct msghdr *msg, size_t len, int flags, int *addr_len);
static atomic_t conn_seq = ATOMIC_INIT(0);

/* --- per-socket mesh table (keyed by sk pointer) -------------------------- */
enum { ST_FREE = 0, ST_PENDING, ST_HANDSHAKING, ST_FAILED };
struct ent { struct sock *sk; __be32 ip; __be16 port; int state; };
#define NENT 64
static struct ent tbl[NENT];
static DEFINE_SPINLOCK(tbl_lock);

static void tbl_add(struct sock *sk, __be32 ip, __be16 port)
{
	int i;
	unsigned long f;

	spin_lock_irqsave(&tbl_lock, f);
	for (i = 0; i < NENT; i++) {
		if (tbl[i].state == ST_FREE) {
			tbl[i].sk = sk; tbl[i].ip = ip; tbl[i].port = port;
			tbl[i].state = ST_PENDING;
			break;
		}
	}
	spin_unlock_irqrestore(&tbl_lock, f);
}
static struct ent *tbl_find(struct sock *sk)
{
	int i;

	for (i = 0; i < NENT; i++)
		if (tbl[i].state != ST_FREE && tbl[i].sk == sk)
			return &tbl[i];
	return NULL;
}
static void tbl_del(struct sock *sk)
{
	int i;
	unsigned long f;

	spin_lock_irqsave(&tbl_lock, f);
	for (i = 0; i < NENT; i++)
		if (tbl[i].sk == sk) { tbl[i].state = ST_FREE; tbl[i].sk = NULL; }
	spin_unlock_irqrestore(&tbl_lock, f);
}
/* Atomically claim PENDING->HANDSHAKING. Returns 1 if we claimed it. */
static int tbl_claim(struct sock *sk, __be32 *ip, __be16 *port)
{
	int i, got = 0;
	unsigned long f;

	spin_lock_irqsave(&tbl_lock, f);
	for (i = 0; i < NENT; i++) {
		if (tbl[i].state == ST_PENDING && tbl[i].sk == sk) {
			tbl[i].state = ST_HANDSHAKING;
			*ip = tbl[i].ip; *port = tbl[i].port;
			got = 1;
			break;
		}
	}
	spin_unlock_irqrestore(&tbl_lock, f);
	return got;
}
static int tbl_state(struct sock *sk)
{
	int i, s = ST_FREE;
	unsigned long f;

	spin_lock_irqsave(&tbl_lock, f);
	for (i = 0; i < NENT; i++)
		if (tbl[i].state != ST_FREE && tbl[i].sk == sk) { s = tbl[i].state; break; }
	spin_unlock_irqrestore(&tbl_lock, f);
	return s;
}

/* --- kallsyms trampoline -------------------------------------------------- */
static unsigned long (*kln)(const char *name);
static int resolve_kln(void)
{
	struct kprobe kp = { .symbol_name = "kallsyms_lookup_name" };
	int r = register_kprobe(&kp);

	if (r < 0)
		return r;
	kln = (void *)kp.addr;
	unregister_kprobe(&kp);
	return kln ? 0 : -ENOENT;
}

/* --- mesh match ----------------------------------------------------------- */
static int parse_u(const char **p, unsigned long *v)
{
	const char *s = *p; unsigned long x = 0;

	if (*s < '0' || *s > '9') return -1;
	while (*s >= '0' && *s <= '9') x = x * 10 + (unsigned long)(*s++ - '0');
	*p = s; *v = x; return 0;
}
static int is_mesh(__be32 be_ip, __be16 be_port)
{
	u32 ip = ntohl(be_ip); unsigned int port = ntohs(be_port); const char *p = mesh;

	while (p && *p) {
		unsigned long o[4], lo, hi; int i;

		for (i = 0; i < 4; i++) {
			if (parse_u(&p, &o[i]) || o[i] > 255) return 0;
			if (i < 3 && *p++ != '.') return 0;
		}
		if (*p++ != ':' || parse_u(&p, &lo)) return 0;
		hi = lo;
		if (*p == '-') { p++; if (parse_u(&p, &hi)) return 0; }
		if (ip == ((o[0] << 24) | (o[1] << 16) | (o[2] << 8) | o[3]) && port >= lo && port <= hi)
			return 1;
		if (*p == ',') p++;
	}
	return 0;
}

/* --- raw TCP I/O on sk, bypassing our own hook (call orig_* directly) ------ */
static int raw_send(struct sock *sk, const void *buf, size_t n)
{
	size_t off = 0;

	while (off < n) {
		struct kvec kv = { .iov_base = (void *)buf + off, .iov_len = n - off };
		struct msghdr m = { .msg_flags = MSG_NOSIGNAL };
		int r;

		iov_iter_kvec(&m.msg_iter, ITER_SOURCE, &kv, 1, n - off);
		r = orig_sendmsg(sk, &m, n - off);
		if (r <= 0) return r ? r : -EPIPE;
		off += r;
	}
	return 0;
}
static int raw_recv(struct sock *sk, void *buf, size_t n, int *eof)
{
	size_t off = 0; int addr_len = 0;

	*eof = 0;
	while (off < n) {
		struct kvec kv = { .iov_base = buf + off, .iov_len = n - off };
		struct msghdr m = { 0 };
		int r;

		iov_iter_kvec(&m.msg_iter, ITER_DEST, &kv, 1, n - off);
		r = orig_recvmsg(sk, &m, n - off, MSG_WAITALL, &addr_len);
		if (r == 0) { *eof = 1; return (int)off; }
		if (r < 0) return r;
		off += r;
	}
	return (int)off;
}
static long read_one_record(struct sock *sk, u8 *buf, size_t cap)
{
	size_t len; int eof, r;

	r = raw_recv(sk, buf, REC_HDR, &eof);
	if (r < 0) return r;
	if (r == 0 && eof) return 0;
	if (r != REC_HDR) return -ECONNRESET;
	len = ((size_t)buf[3] << 8) | buf[4];
	if (len > REC_MAX_CT || REC_HDR + len > cap) return -EPROTO;
	if (len) {
		r = raw_recv(sk, buf + REC_HDR, len, &eof);
		if (r < 0) return r;
		if (eof || (size_t)r != len) return -ECONNRESET;
	}
	return (long)(REC_HDR + len);
}

/* --- vsock frames (separate socket, not our hook) ------------------------- */
static int vs_send(struct socket *vs, const void *buf, size_t n)
{
	size_t off = 0;

	while (off < n) {
		struct kvec v = { .iov_base = (void *)buf + off, .iov_len = n - off };
		struct msghdr m = { .msg_flags = MSG_NOSIGNAL };
		int r = kernel_sendmsg(vs, &m, &v, 1, n - off);

		if (r <= 0) return r ? r : -EPIPE;
		off += r;
	}
	return 0;
}
static int vs_recv(struct socket *vs, void *buf, size_t n, int *eof)
{
	size_t off = 0;

	*eof = 0;
	while (off < n) {
		struct kvec v = { .iov_base = buf + off, .iov_len = n - off };
		struct msghdr m = { 0 };
		int r = kernel_recvmsg(vs, &m, &v, 1, n - off, MSG_WAITALL);

		if (r == 0) { *eof = 1; return (int)off; }
		if (r < 0) return r;
		off += r;
	}
	return (int)off;
}
static int frame_send(struct socket *vs, u8 t, const void *p, u32 len)
{
	u8 hdr[5] = { t, (u8)(len >> 24), (u8)(len >> 16), (u8)(len >> 8), (u8)len };
	int r = vs_send(vs, hdr, 5);

	if (!r && len) r = vs_send(vs, p, len);
	return r;
}
static long frame_recv(struct socket *vs, u8 *t, u8 *buf, size_t cap)
{
	u8 hdr[5]; u32 len; int eof, r = vs_recv(vs, hdr, 5, &eof);

	if (r < 0) return r;
	if (eof || r != 5) return -ECONNRESET;
	len = ((u32)hdr[1] << 24) | ((u32)hdr[2] << 16) | ((u32)hdr[3] << 8) | hdr[4];
	if (len > cap) return -EMSGSIZE;
	if (len) {
		r = vs_recv(vs, buf, len, &eof);
		if (r < 0) return r;
		if (eof || (u32)r != len) return -ECONNRESET;
	}
	*t = hdr[0];
	return (long)len;
}

/* --- kTLS install --------------------------------------------------------- */
struct dir_secret { u64 seq; u8 key[16]; u8 iv12[12]; };
static int parse_secrets(const u8 *p, u32 len, struct dir_secret *tx, struct dir_secret *rx,
			 char *peer, size_t peercap)
{
	u32 off = 2; int d; struct dir_secret *dir;

	if (len < 2) return -EINVAL;
	for (d = 0; d < 2; d++) {
		int i;

		dir = d == 0 ? tx : rx;
		if (off + 8 > len) return -EINVAL;
		dir->seq = 0;
		for (i = 0; i < 8; i++) dir->seq = (dir->seq << 8) | p[off + i];
		off += 8;
		if (off + 1 > len || p[off] != 32 || off + 1 + 32 > len) return -EINVAL;
		off += 1 + 32;
		if (off + 1 > len || p[off] != 16 || off + 1 + 16 > len) return -EINVAL;
		off += 1; memcpy(dir->key, p + off, 16); off += 16;
		if (off + 12 > len) return -EINVAL;
		memcpy(dir->iv12, p + off, 12); off += 12;
	}
	if (off + 2 <= len) {
		u32 pl = ((u32)p[off] << 8) | p[off + 1];

		off += 2;
		if (off + pl <= len && pl < peercap) { memcpy(peer, p + off, pl); peer[pl] = 0; }
	}
	return 0;
}
static void fill_ci(struct tls12_crypto_info_aes_gcm_128 *ci, const struct dir_secret *d)
{
	int i;

	memset(ci, 0, sizeof(*ci));
	ci->info.version = TLS_1_3_VERSION;
	ci->info.cipher_type = TLS_CIPHER_AES_GCM_128;
	memcpy(ci->key, d->key, 16);
	memcpy(ci->salt, d->iv12, 4);
	memcpy(ci->iv, d->iv12 + 4, 8);
	for (i = 0; i < 8; i++) ci->rec_seq[i] = (u8)(d->seq >> (8 * (7 - i)));
}
static int install_ktls(struct sock *sk, const struct dir_secret *tx, const struct dir_secret *rx)
{
	struct tls12_crypto_info_aes_gcm_128 ci;
	int err;

	lock_sock(sk);
	err = p_tcp_set_ulp(sk, "tls");
	release_sock(sk);
	if (err) { pr_err(PFX "tcp_set_ulp(tls) err=%d\n", err); return err; }
	fill_ci(&ci, tx);
	err = sk->sk_prot->setsockopt(sk, SOL_TLS, TLS_TX, KERNEL_SOCKPTR(&ci), sizeof(ci));
	if (err) { pr_err(PFX "TLS_TX err=%d\n", err); return err; }
	fill_ci(&ci, rx);
	err = sk->sk_prot->setsockopt(sk, SOL_TLS, TLS_RX, KERNEL_SOCKPTR(&ci), sizeof(ci));
	if (err) { pr_err(PFX "TLS_RX err=%d\n", err); return err; }
	return 0;
}

/* --- the hold: relay handshake + install kTLS (NOT under sk lock) ---------- */
static int do_handshake(struct sock *sk, __be32 be_ip, __be16 be_port, unsigned int id)
{
	struct socket *vs = NULL;
	struct sockaddr_vm svm;
	u8 *fbuf = NULL, *rbuf = NULL, open[7], t;
	struct dir_secret tx, rx;
	char peer[128] = "";
	long n; int err, eof;
	unsigned int to_peer = 0, need_peer = 0;

	fbuf = kmalloc(MAX_FRAME, GFP_KERNEL);
	rbuf = kmalloc(REC_HDR + REC_MAX_CT, GFP_KERNEL);
	if (!fbuf || !rbuf) { err = -ENOMEM; goto out; }

	err = sock_create_kern(sock_net(sk), AF_VSOCK, SOCK_STREAM, 0, &vs);
	if (err) { pr_err(PFX "[c%u] vsock create err=%d\n", id, err); goto out; }
	memset(&svm, 0, sizeof(svm));
	svm.svm_family = AF_VSOCK; svm.svm_cid = agent_cid; svm.svm_port = agent_port;
	err = kernel_connect(vs, (struct sockaddr_unsized *)&svm, sizeof(svm), 0);
	if (err) { pr_err(PFX "[c%u] vsock connect cid=%u port=%u err=%d\n", id, agent_cid, agent_port, err); goto out; }
	pr_info(PFX "[c%u] HOLD on first I/O: mesh %pI4:%u, relaying TLS handshake over vsock to cid=%u port=%u\n",
		id, &be_ip, ntohs(be_port), agent_cid, agent_port);

	open[0] = 4; memcpy(open + 1, &be_ip, 4); memcpy(open + 5, &be_port, 2);
	err = frame_send(vs, T_OPEN, open, 7);
	if (err) goto out;

	for (;;) {
		n = frame_recv(vs, &t, fbuf, MAX_FRAME);
		if (n < 0) { err = (int)n; pr_err(PFX "[c%u] frame_recv err=%ld\n", id, n); goto out; }
		if (t == T_TO_PEER) {
			to_peer++;
			err = raw_send(sk, fbuf, n);
			if (err) { pr_err(PFX "[c%u] write TLS to peer err=%d\n", id, err); goto out; }
		} else if (t == T_NEED_PEER) {
			long rn = read_one_record(sk, rbuf, REC_HDR + REC_MAX_CT);

			need_peer++;
			if (rn < 0) { err = (int)rn; pr_err(PFX "[c%u] read record err=%ld\n", id, rn); goto out; }
			err = frame_send(vs, T_FROM_PEER, rbuf, (u32)rn);
			if (err) goto out;
			if (rn == 0) { err = -ECONNRESET; goto out; }
		} else if (t == T_SECRETS) {
			err = parse_secrets(fbuf, (u32)n, &tx, &rx, peer, sizeof(peer));
			if (err) { pr_err(PFX "[c%u] bad SECRETS err=%d\n", id, err); goto out; }
			break;
		} else if (t == T_DENY) {
			fbuf[min_t(long, n, MAX_FRAME - 1)] = 0;
			pr_warn(PFX "[c%u] DENY from relay: %s -> first I/O returns -EACCES, no app bytes on wire\n", id, fbuf);
			err = -EACCES; goto out;
		} else {
			fbuf[min_t(long, n, MAX_FRAME - 1)] = 0;
			pr_warn(PFX "[c%u] relay frame 0x%02x: %s\n", id, t, fbuf);
			err = -ECONNABORTED; goto out;
		}
	}
	sock_release(vs); vs = NULL;
	err = install_ktls(sk, &tx, &rx);
	if (err) goto out_noclose;
	pr_info(PFX "[c%u] HANDSHAKE DONE peer=%s; kTLS installed on the workload socket (TO_PEER=%u NEED_PEER=%u); kernel carries TLS, relay out of path\n",
		id, peer[0] ? peer : "?", to_peer, need_peer);
	err = 0;
	goto out_noclose;
out:
	if (err == -EACCES || err)
		kernel_sock_shutdown(sk->sk_socket, SHUT_RDWR);
	(void)eof;
out_noclose:
	if (vs) sock_release(vs);
	kfree(fbuf); kfree(rbuf);
	return err;
}

/* Runs the handshake once for a PENDING sk; returns 0 if kTLS is ready
 * (sk->sk_prot is now the tls proto), or a negative errno. */
static int maybe_handshake(struct sock *sk)
{
	__be32 ip; __be16 port; unsigned int id; int err;

	if (!tbl_claim(sk, &ip, &port))
		return 1; /* not ours / not pending (someone else handled / in flight) */
	id = atomic_inc_return(&conn_seq);
	err = do_handshake(sk, ip, port, id);
	tbl_del(sk);
	return err; /* 0 = kTLS ready; <0 = failed */
}

static int hooked_sendmsg(struct sock *sk, struct msghdr *msg, size_t size)
{
	int s = tbl_state(sk), err;

	if (s == ST_FREE)
		return orig_sendmsg(sk, msg, size);
	if (s == ST_HANDSHAKING)
		return orig_sendmsg(sk, msg, size); /* our own raw I/O re-entry (defensive) */
	/* ST_PENDING: run the hold */
	err = maybe_handshake(sk);
	if (err == 1)
		return orig_sendmsg(sk, msg, size);
	if (err < 0)
		return err; /* DENY/handshake failure -> app write fails, nothing sent */
	return sk->sk_prot->sendmsg(sk, msg, size); /* sk_prot is now tls */
}
static int hooked_recvmsg(struct sock *sk, struct msghdr *msg, size_t len, int flags, int *addr_len)
{
	int s = tbl_state(sk), err;

	if (s == ST_FREE)
		return orig_recvmsg(sk, msg, len, flags, addr_len);
	if (s == ST_HANDSHAKING)
		return orig_recvmsg(sk, msg, len, flags, addr_len);
	err = maybe_handshake(sk);
	if (err == 1)
		return orig_recvmsg(sk, msg, len, flags, addr_len);
	if (err < 0)
		return err;
	return sk->sk_prot->recvmsg(sk, msg, len, flags, addr_len);
}

static int hooked_connect(struct sock *sk, struct sockaddr_unsized *uaddr, int addr_len)
{
	int ret = orig_connect(sk, uaddr, addr_len);
	struct sockaddr_in *sin = (struct sockaddr_in *)uaddr;

	if (ret != 0)
		return ret;
	if (!uaddr || addr_len < (int)sizeof(*sin) || sin->sin_family != AF_INET)
		return 0;
	if (is_mesh(sin->sin_addr.s_addr, sin->sin_port))
		tbl_add(sk, sin->sin_addr.s_addr, sin->sin_port); /* handshake on first I/O */
	return 0;
}

static int __init igkmg_init(void)
{
	pr_info(PFX "init on stock kernel; mesh=\"%s\" agent cid=%u port=%u\n",
		mesh ? mesh : "", agent_cid, agent_port);
	if (resolve_kln()) { pr_err(PFX "kallsyms resolve failed\n"); return -ENOENT; }
	tcp_prot_p = (struct proto *)kln("tcp_prot");
	p_tcp_set_ulp = (void *)kln("tcp_set_ulp");
	if (!tcp_prot_p || !p_tcp_set_ulp) { pr_err(PFX "symbol resolve failed\n"); return -ENOENT; }
	orig_connect = tcp_prot_p->connect;
	orig_sendmsg = tcp_prot_p->sendmsg;
	orig_recvmsg = tcp_prot_p->recvmsg;
	WRITE_ONCE(tcp_prot_p->sendmsg, hooked_sendmsg);
	WRITE_ONCE(tcp_prot_p->recvmsg, hooked_recvmsg);
	WRITE_ONCE(tcp_prot_p->connect, hooked_connect);
	pr_info(PFX "hooked tcp_prot connect/sendmsg/recvmsg\n");
	return 0;
}
static void __exit igkmg_exit(void)
{
	if (tcp_prot_p) {
		WRITE_ONCE(tcp_prot_p->connect, orig_connect);
		WRITE_ONCE(tcp_prot_p->sendmsg, orig_sendmsg);
		WRITE_ONCE(tcp_prot_p->recvmsg, orig_recvmsg);
	}
	synchronize_rcu();
	msleep(100);
	pr_info(PFX "unhooked; exit\n");
}
module_init(igkmg_init);
module_exit(igkmg_exit);
