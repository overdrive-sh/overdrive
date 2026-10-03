// SPDX-License-Identifier: GPL-2.0
/*
 * Spike E (Overdrive GH #303, increment-j) FULL MODULE. THROWAWAY.
 *
 * Extends Spike C/D (increment-h/-i: outbound client slice + resolve/LB over
 * vsock) with the two gaps #303 Spike C deferred for a Linux kernel module:
 *
 *   PART 1 -- the inbound accept()/server path. Hooks tcp_prot.accept
 *   (== inet_csk_accept). When a mesh LISTENER (a local port in `mesh_listen`)
 *   accepts an inbound connection, the newly-accepted child socket is tagged
 *   ROLE_SERVER and gets server-side transparent mTLS: on first I/O the module
 *   relays the SERVER-role TLS 1.3 handshake over vsock to the host agent --
 *   sending the ACCEPT frame (0x08) that selects the rustls SERVER role holding
 *   the guest-server SVID with client-cert-required -- then installs kTLS on the
 *   accepted socket exactly as the client path does. The server workload reads
 *   and writes a plaintext-looking socket; the kernel carries the record layer.
 *
 *   PART 2 -- non-blocking / epoll readiness. A blocking connect()/accept()
 *   thread can run the handshake inline (Spike C). A non-blocking workload sits
 *   in epoll_wait and never calls into the socket, so NOTHING drives the
 *   multi-round-trip vsock handshake. This module drives it from a per-socket
 *   KERNEL THREAD (kthread) kicked at connect()/accept() time, and WITHHOLDS
 *   readiness until kTLS is installed by swapping the socket's proto_ops to a
 *   private copy whose ->poll masks EPOLLIN/EPOLLOUT while handshaking (the
 *   Linux-kTLS analogue of the Unikraft B2 lib-lwip readiness hook). On
 *   completion the kthread installs kTLS and wakes the poll waiters; on DENY it
 *   sets sk_err (surfaced to the app via SO_ERROR) and reports the error.
 *
 * Camblet-style in-guest transparent mTLS as an out-of-tree LKM on a STOCK,
 * UNMODIFIED Linux kernel (7.0.0-29-generic). Hooks tcp_prot
 * connect/accept/sendmsg/recvmsg + swaps per-socket proto_ops.poll. No record
 * layer, no crypto, no KeyUpdate handling in the module -- the kernel's kTLS
 * does all of that. The SVID private key never crosses vsock.
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
#include <linux/kthread.h>
#include <linux/sched.h>
#include <linux/wait.h>
#include <linux/poll.h>
#include <linux/fs.h>
#include <linux/fcntl.h>
#include <net/sock.h>
#include <net/tcp.h>
#include <net/tls.h>
#include <net/inet_sock.h>
#include <uapi/linux/vm_sockets.h>

MODULE_LICENSE("GPL");
MODULE_AUTHOR("Overdrive spike");
MODULE_DESCRIPTION("GH#303 Spike E in-guest transparent-mTLS LKM: accept() + non-blocking/epoll (throwaway)");

#define PFX "IGKME: "

/* mesh = client VIPs the app dials (resolve+rewrite at connect, Spike D).
 * mesh_listen = local LISTEN ports that take inbound server-side mTLS (Spike E).*/
static char *mesh;
module_param(mesh, charp, 0444);
static char *mesh_listen;
module_param(mesh_listen, charp, 0444);
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
#define T_ACCEPT 0x08	/* guest->host: family·lip·lport·rip·rport (13 B) -- SERVER role */
#define T_RESOLVE 0x09	/* guest->host: family·vip_ip·vip_port (Spike D) */
#define T_RESOLVED 0x0a	/* host->guest: family·backend_ip·backend_port·peer_id_len·peer_id */
#define MAX_FRAME (64 * 1024)
#define REC_HDR 5
#define REC_MAX_CT (16384 + 256)

enum { ROLE_CLIENT = 0, ROLE_SERVER = 1 };

static struct proto *tcp_prot_p;
static int (*p_tcp_set_ulp)(struct sock *sk, const char *name);
static int (*orig_connect)(struct sock *sk, struct sockaddr_unsized *uaddr, int addr_len);
static struct sock *(*orig_accept)(struct sock *sk, struct proto_accept_arg *arg);
static int (*orig_sendmsg)(struct sock *sk, struct msghdr *msg, size_t size);
static int (*orig_recvmsg)(struct sock *sk, struct msghdr *msg, size_t len, int flags, int *addr_len);
static atomic_t conn_seq = ATOMIC_INIT(0);

/* Per-socket proto_ops copy: identical to inet_stream_ops except ->poll, which
 * masks readiness while a mesh socket is still handshaking (Part 2). We swap a
 * mesh socket's struct socket ->ops pointer to this; the pointer field is
 * writable (only the pointee is const), so no rodata patching. */
static struct proto_ops igkm_stream_ops;
static __poll_t (*orig_poll)(struct file *file, struct socket *sock, struct poll_table_struct *wait);

/* --- per-socket mesh table (keyed by sk pointer) -------------------------- */
enum { ST_FREE = 0, ST_PENDING, ST_HANDSHAKING, ST_READY, ST_FAILED };
struct ent {
	struct sock *sk;
	int state;
	int role;		/* ROLE_CLIENT / ROLE_SERVER */
	int kd;			/* kthread-driven (non-blocking): the kthread owns the handshake */
	int err;		/* positive errno to report on ST_FAILED (e.g. EACCES) */
	__be32 ip; __be16 port;			/* CLIENT: chosen backend dst (OPEN) */
	__be32 lsaddr, rdaddr; __be16 lsport, rdport;	/* SERVER: ACCEPT frame addrs */
	char peer[128];		/* CLIENT: expected backend SPIFFE ID RESOLVE promised */
	struct task_struct *kth;
	wait_queue_head_t wq;
};
#define NENT 64
static struct ent tbl[NENT];
static DEFINE_SPINLOCK(tbl_lock);

static struct ent *find_locked(struct sock *sk)
{
	int i;

	for (i = 0; i < NENT; i++)
		if (tbl[i].state != ST_FREE && tbl[i].sk == sk)
			return &tbl[i];
	return NULL;
}

/* Record a new mesh socket as PENDING. role/kd/addrs set by the caller fields.
 * First evict any STALE entry for this sk pointer: the kernel reuses a freed
 * struct sock pointer for a new socket, and a terminal (READY/FAILED) entry
 * left over from a prior connection on the same pointer would mis-serve the new
 * socket (the stale-READY -> forward-via-hooked-tcp_prot self-recursion that
 * overflowed the stack in run 0021). An entry only outlives its socket because
 * a kTLS socket's close bypasses our tcp_prot hook (sk_prot is tls); evicting
 * on re-add keeps the table correct without a close hook. */
static struct ent *tbl_add(struct sock *sk, int role)
{
	int i;
	unsigned long f;
	struct ent *e = NULL;

	spin_lock_irqsave(&tbl_lock, f);
	for (i = 0; i < NENT; i++)
		if (tbl[i].sk == sk) { tbl[i].state = ST_FREE; tbl[i].sk = NULL; }
	for (i = 0; i < NENT; i++) {
		if (tbl[i].state == ST_FREE) {
			e = &tbl[i];
			memset(e, 0, sizeof(*e));
			e->sk = sk;
			e->role = role;
			e->state = ST_PENDING;
			init_waitqueue_head(&e->wq);
			break;
		}
	}
	spin_unlock_irqrestore(&tbl_lock, f);
	return e;
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
/* Snapshot the entry for a reader without claiming it. Returns 1 if present. */
static int tbl_get(struct sock *sk, struct ent *out)
{
	unsigned long f;
	struct ent *e;
	int got = 0;

	spin_lock_irqsave(&tbl_lock, f);
	e = find_locked(sk);
	if (e) { *out = *e; got = 1; }
	spin_unlock_irqrestore(&tbl_lock, f);
	return got;
}
static int tbl_state(struct sock *sk)
{
	unsigned long f;
	struct ent *e;
	int s = ST_FREE;

	spin_lock_irqsave(&tbl_lock, f);
	e = find_locked(sk);
	if (e) s = e->state;
	spin_unlock_irqrestore(&tbl_lock, f);
	return s;
}
/* Atomically claim PENDING->HANDSHAKING (the inline blocking-client path). */
static int tbl_claim(struct sock *sk, struct ent *out)
{
	unsigned long f;
	struct ent *e;
	int got = 0;

	spin_lock_irqsave(&tbl_lock, f);
	e = find_locked(sk);
	if (e && e->state == ST_PENDING) {
		e->state = ST_HANDSHAKING;
		*out = *e;
		got = 1;
	}
	spin_unlock_irqrestore(&tbl_lock, f);
	return got;
}
/* Terminal transition (ST_READY / ST_FAILED) + wake poll/IO waiters. */
static void tbl_finish(struct sock *sk, int state, int err)
{
	unsigned long f;
	struct ent *e;
	wait_queue_head_t *wq = NULL;

	spin_lock_irqsave(&tbl_lock, f);
	e = find_locked(sk);
	if (e) { e->state = state; e->err = err; wq = &e->wq; }
	spin_unlock_irqrestore(&tbl_lock, f);
	if (wq)
		wake_up_interruptible_all(wq);
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
/* mesh_listen is a comma list of local LISTEN ports (host order). */
static int is_mesh_listen(unsigned int lport)
{
	const char *p = mesh_listen;

	while (p && *p) {
		unsigned long lo, hi;

		if (parse_u(&p, &lo)) return 0;
		hi = lo;
		if (*p == '-') { p++; if (parse_u(&p, &hi)) return 0; }
		if (lport >= lo && lport <= hi) return 1;
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

/* --- Spike D: resolve a VIP to a backend over vsock (UNDER the connect lock) */
static int do_resolve(struct sock *sk, __be32 vip, __be16 vport,
		      __be32 *bip, __be16 *bport, char *peer, size_t peercap)
{
	struct socket *vs = NULL;
	struct sockaddr_vm svm;
	u8 *fbuf = NULL, req[7], t;
	long n;
	int err;

	fbuf = kmalloc(MAX_FRAME, GFP_KERNEL);
	if (!fbuf) { err = -ENOMEM; goto out; }
	err = sock_create_kern(sock_net(sk), AF_VSOCK, SOCK_STREAM, 0, &vs);
	if (err) { pr_err(PFX "[resolve] vsock create err=%d\n", err); goto out; }
	memset(&svm, 0, sizeof(svm));
	svm.svm_family = AF_VSOCK; svm.svm_cid = agent_cid; svm.svm_port = agent_port;
	err = kernel_connect(vs, (struct sockaddr_unsized *)&svm, sizeof(svm), 0);
	if (err) { pr_err(PFX "[resolve] vsock connect cid=%u port=%u err=%d\n", agent_cid, agent_port, err); goto out; }
	pr_info(PFX "[resolve] vip %pI4:%u -> asking agent over vsock cid=%u port=%u\n",
		&vip, ntohs(vport), agent_cid, agent_port);

	req[0] = 4; memcpy(req + 1, &vip, 4); memcpy(req + 5, &vport, 2);
	err = frame_send(vs, T_RESOLVE, req, 7);
	if (err) goto out;

	n = frame_recv(vs, &t, fbuf, MAX_FRAME);
	if (n < 0) { err = (int)n; pr_err(PFX "[resolve] frame_recv err=%ld\n", n); goto out; }
	if (t == T_RESOLVED) {
		if (n < 7 || fbuf[0] != 4) { err = -EPROTO; goto out; }
		memcpy(bip, fbuf + 1, 4);
		memcpy(bport, fbuf + 5, 2);
		peer[0] = 0;
		if (n >= 9) {
			u32 pl = ((u32)fbuf[7] << 8) | fbuf[8];

			if (9 + pl <= (u32)n && pl < peercap) { memcpy(peer, fbuf + 9, pl); peer[pl] = 0; }
		}
		pr_info(PFX "[resolve] agent chose backend %pI4:%u expected_peer=%s; rewriting connect dst\n",
			bip, ntohs(*bport), peer[0] ? peer : "?");
		err = 0;
	} else if (t == T_DENY) {
		fbuf[min_t(long, n, MAX_FRAME - 1)] = 0;
		pr_warn(PFX "[resolve] vip %pI4:%u DENY from agent: %s -> connect() returns -EACCES, no TCP, no app bytes\n",
			&vip, ntohs(vport), fbuf);
		err = -EACCES;
	} else {
		fbuf[min_t(long, n, MAX_FRAME - 1)] = 0;
		pr_warn(PFX "[resolve] vip %pI4:%u agent frame 0x%02x: %s -> connect fails\n", &vip, ntohs(vport), t, fbuf);
		err = -EHOSTUNREACH;
	}
out:
	if (vs) sock_release(vs);
	kfree(fbuf);
	return err;
}

/* --- the hold: relay handshake + install kTLS (NOT under sk lock) ----------
 * role selects the opening frame: ROLE_CLIENT sends OPEN(dst) and the agent is
 * the rustls CLIENT; ROLE_SERVER sends ACCEPT(local,remote) and the agent is
 * the rustls SERVER (guest-server SVID, client cert required). The relay loop
 * (TO_PEER / NEED_PEER / SECRETS / DENY) is identical for both roles. */
static int do_handshake(struct ent *c, unsigned int id)
{
	struct sock *sk = c->sk;
	struct socket *vs = NULL;
	struct sockaddr_vm svm;
	u8 *fbuf = NULL, *rbuf = NULL, open[13], t;
	struct dir_secret tx, rx;
	char peer[128] = "";
	long n; int err, eof;
	unsigned int to_peer = 0, need_peer = 0;
	u8 of; u32 olen;

	fbuf = kmalloc(MAX_FRAME, GFP_KERNEL);
	rbuf = kmalloc(REC_HDR + REC_MAX_CT, GFP_KERNEL);
	if (!fbuf || !rbuf) { err = -ENOMEM; goto out; }

	err = sock_create_kern(sock_net(sk), AF_VSOCK, SOCK_STREAM, 0, &vs);
	if (err) { pr_err(PFX "[c%u] vsock create err=%d\n", id, err); goto out; }
	memset(&svm, 0, sizeof(svm));
	svm.svm_family = AF_VSOCK; svm.svm_cid = agent_cid; svm.svm_port = agent_port;
	err = kernel_connect(vs, (struct sockaddr_unsized *)&svm, sizeof(svm), 0);
	if (err) { pr_err(PFX "[c%u] vsock connect cid=%u port=%u err=%d\n", id, agent_cid, agent_port, err); goto out; }

	if (c->role == ROLE_SERVER) {
		open[0] = 4;
		memcpy(open + 1, &c->lsaddr, 4); memcpy(open + 5, &c->lsport, 2);
		memcpy(open + 7, &c->rdaddr, 4); memcpy(open + 11, &c->rdport, 2);
		of = T_ACCEPT; olen = 13;
		pr_info(PFX "[c%u] HOLD on first I/O: SERVER accept() local %pI4:%u <- remote %pI4:%u, relaying SERVER TLS handshake over vsock cid=%u port=%u\n",
			id, &c->lsaddr, ntohs(c->lsport), &c->rdaddr, ntohs(c->rdport), agent_cid, agent_port);
	} else {
		open[0] = 4; memcpy(open + 1, &c->ip, 4); memcpy(open + 5, &c->port, 2);
		of = T_OPEN; olen = 7;
		pr_info(PFX "[c%u] HOLD on first I/O: CLIENT mesh %pI4:%u, relaying CLIENT TLS handshake over vsock cid=%u port=%u\n",
			id, &c->ip, ntohs(c->port), agent_cid, agent_port);
	}
	err = frame_send(vs, of, open, olen);
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
	pr_info(PFX "[c%u] HANDSHAKE DONE role=%s peer=%s; kTLS installed on the workload socket (TO_PEER=%u NEED_PEER=%u); kernel carries TLS, relay out of path\n",
		id, c->role == ROLE_SERVER ? "server" : "client", peer[0] ? peer : "?", to_peer, need_peer);
	if (c->role == ROLE_SERVER)
		pr_info(PFX "[c%u] SERVER-IDENTITY verified-client-peer=%s (the inbound caller the agent authenticated for this accept())\n",
			id, peer[0] ? peer : "?");
	else
		pr_info(PFX "[c%u] RESOLVE-MATCH expected_peer=%s handshake_peer=%s match=%d\n",
			id, c->peer[0] ? c->peer : "?", peer[0] ? peer : "?",
			(c->peer[0] && peer[0] && !strcmp(c->peer, peer)) ? 1 : 0);
	err = 0;
	goto out_noclose;
out:
	if (err)
		kernel_sock_shutdown(sk->sk_socket, SHUT_RDWR);
	(void)eof;
out_noclose:
	if (vs) sock_release(vs);
	kfree(fbuf); kfree(rbuf);
	return err;
}

/* --- per-socket proto_ops swap + readiness-masking poll (Part 2) ----------- */
static void igkm_swap_ops(struct sock *sk)
{
	struct socket *so = sk->sk_socket;

	if (so && so->ops != &igkm_stream_ops)
		so->ops = &igkm_stream_ops;	/* pointer field is writable; pointee const */
}
/* While a mesh socket is PENDING/HANDSHAKING we register the poll_wait (so the
 * app re-polls on our wakeup) but MASK OUT readiness, so a non-blocking
 * connect()/accept() app sitting in epoll_wait is NOT told the socket is
 * readable/writable before kTLS is installed. On FAILED we add EPOLLERR|HUP so
 * the app wakes and reads SO_ERROR. READY/FREE pass through to tcp_poll (the
 * kTLS socket then polls normally). */
static __poll_t igkm_poll(struct file *file, struct socket *sock, struct poll_table_struct *wait)
{
	__poll_t mask = orig_poll(file, sock, wait);
	int st = tbl_state(sock->sk);

	if (st == ST_PENDING || st == ST_HANDSHAKING)
		return mask & ~(EPOLLIN | EPOLLOUT | EPOLLRDNORM | EPOLLWRNORM |
				EPOLLRDBAND | EPOLLWRBAND | EPOLLPRI);
	if (st == ST_FAILED)
		return mask | EPOLLERR | EPOLLHUP;
	return mask;
}

/* --- the per-socket background handshake driver (kthread, Part 2) ----------
 * Runs the whole vsock handshake while the workload sits in epoll_wait and
 * never calls into the socket. For a CLIENT it first waits for TCP to reach
 * ESTABLISHED (connect() returned EINPROGRESS); for a SERVER the child is
 * already ESTABLISHED. It then runs do_handshake + installs kTLS, and wakes the
 * poll waiters (readiness now un-masked) or, on DENY/failure, sets sk_err
 * (surfaced via SO_ERROR) and reports the error. Nothing here holds the sk
 * lock across the vsock round trips; install_ktls takes it briefly. */
static int igkm_drive(void *data)
{
	struct sock *sk = data;
	struct ent snap;
	int err, i;

	if (!tbl_get(sk, &snap))
		return 0;

	if (snap.role == ROLE_SERVER) {
		int j;

		/* The accept hook returned the child; inet_accept grafts its struct
		 * socket right after. Wait briefly for the graft, then mask the
		 * child's ->poll so a non-blocking accept()+epoll app is not told the
		 * socket is readable before kTLS. (Future children are already
		 * born-masked via the listener ops swap in the accept hook.) */
		for (j = 0; j < 1000 && !READ_ONCE(sk->sk_socket); j++)
			msleep(1);
		igkm_swap_ops(sk);
	}

	if (snap.role == ROLE_CLIENT) {
		for (i = 0; i < 10000; i++) {	/* up to ~10s for the TCP handshake */
			int s = READ_ONCE(sk->sk_state);

			if (s == TCP_ESTABLISHED) break;
			if (s == TCP_CLOSE || s == TCP_CLOSE_WAIT || s == TCP_CLOSING) break;
			if (kthread_should_stop()) return 0;
			msleep(1);
		}
		if (READ_ONCE(sk->sk_state) != TCP_ESTABLISHED) {
			pr_warn(PFX "[kthread] client sk not ESTABLISHED (state=%d) -> fail\n", sk->sk_state);
			WRITE_ONCE(sk->sk_err, ECONNREFUSED);
			tbl_finish(sk, ST_FAILED, ECONNREFUSED);
			if (sk->sk_error_report) sk->sk_error_report(sk);
			return 0;
		}
	}

	{
		unsigned long f;
		struct ent *e;
		struct ent run;
		int claimed = 0;

		spin_lock_irqsave(&tbl_lock, f);
		e = find_locked(sk);
		if (e && e->state == ST_PENDING) { e->state = ST_HANDSHAKING; run = *e; claimed = 1; }
		spin_unlock_irqrestore(&tbl_lock, f);
		if (!claimed)
			return 0;
		err = do_handshake(&run, atomic_inc_return(&conn_seq));
	}

	if (err == 0) {
		pr_info(PFX "[kthread] role=%s background handshake DONE; kTLS installed, waking poll (readiness now reported)\n",
			snap.role == ROLE_SERVER ? "server" : "client");
		tbl_finish(sk, ST_READY, 0);
		/* wake epoll/poll waiters: re-poll now returns EPOLLOUT/EPOLLIN */
		if (sk->sk_state_change) sk->sk_state_change(sk);
		if (sk->sk_write_space)  sk->sk_write_space(sk);
		if (sk->sk_data_ready)   sk->sk_data_ready(sk);
	} else {
		int e = (err == -EACCES) ? EACCES : -err;

		if (e <= 0) e = ECONNRESET;
		pr_warn(PFX "[kthread] role=%s background handshake FAILED err=%d -> sk_err=%d (app sees EPOLLERR / SO_ERROR), no app bytes\n",
			snap.role == ROLE_SERVER ? "server" : "client", err, e);
		WRITE_ONCE(sk->sk_err, e);
		tbl_finish(sk, ST_FAILED, e);
		if (sk->sk_error_report) sk->sk_error_report(sk);
	}
	return 0;
}

static int spawn_driver(struct sock *sk, const char *tag)
{
	struct task_struct *t;
	unsigned long f;
	struct ent *e;

	t = kthread_run(igkm_drive, sk, "igkme-hs");
	if (IS_ERR(t)) {
		pr_err(PFX "kthread_run(%s) failed %ld\n", tag, PTR_ERR(t));
		return (int)PTR_ERR(t);
	}
	spin_lock_irqsave(&tbl_lock, f);
	e = find_locked(sk);
	if (e) e->kth = t;
	spin_unlock_irqrestore(&tbl_lock, f);
	return 0;
}

/* --- is this I/O call non-blocking? --------------------------------------- */
static int call_nonblocking(struct sock *sk, int msg_flags)
{
	if (msg_flags & MSG_DONTWAIT)
		return 1;
	if (sk->sk_socket && sk->sk_socket->file && (sk->sk_socket->file->f_flags & O_NONBLOCK))
		return 1;
	return 0;
}

/* Wait for a kthread-driven socket to reach a terminal state. The entry (and
 * its wq) persists through READY/FAILED for a kd socket, so the wq stays valid.
 * Returns 0 on READY (caller forwards via sk->sk_prot=tls), <0 on FAILED. */
static int wait_for_driver(struct sock *sk)
{
	wait_queue_head_t *wq = NULL;
	unsigned long f;
	struct ent *e;
	struct ent snap;
	int st;

	spin_lock_irqsave(&tbl_lock, f);
	e = find_locked(sk);
	if (e) wq = &e->wq;
	spin_unlock_irqrestore(&tbl_lock, f);
	if (!wq)
		return 1;
	wait_event_interruptible(*wq, (st = tbl_state(sk)) == ST_READY || st == ST_FAILED);
	st = tbl_state(sk);
	if (st == ST_READY)
		return 0;
	if (tbl_get(sk, &snap))
		return -snap.err;
	return -ECONNRESET;
}

/* --- drive the hold on first I/O (shared by sendmsg/recvmsg) ---------------
 * Called for every non-FREE mesh socket. Note: our own raw handshake I/O (both
 * the inline path and the kthread) goes through orig_sendmsg/orig_recvmsg
 * DIRECTLY, bypassing these hooks -- so the only caller that reaches io_gate
 * mid-handshake is the APP itself. Returns: 1 = pass to orig; 0 = kTLS ready,
 * caller forwards via sk->sk_prot=tls; <0 = errno to return to the app. */
static int io_gate(struct sock *sk, int caller_nb)
{
	struct ent snap;

	if (!tbl_get(sk, &snap))
		return 1;				/* FREE: not ours */
	if (snap.state == ST_READY) {
		/* A genuinely-kTLS socket has sk_prot == tls and never enters this
		 * hook; so a READY entry reached HERE is always STALE (the sk pointer
		 * was reused by a fresh, non-kTLS socket). Evict and pass through --
		 * returning 0 would forward via the hooked tcp_prot and self-recurse. */
		tbl_del(sk);
		return 1;
	}
	if (snap.state == ST_FAILED)
		return -snap.err;
	if (snap.kd) {
		/* kthread-driven (non-blocking client at connect, or any server
		 * accept): the kthread owns the handshake; this syscall must not
		 * touch the socket. Non-blocking caller -> EAGAIN; blocking caller
		 * (e.g. a blocking server accept()+read()) -> wait for the kthread. */
		if (caller_nb)
			return -EAGAIN;
		return wait_for_driver(sk);
	}
	/* ST_PENDING, inline blocking-client path (Spike C/D regression): the
	 * app's own blocking syscall runs the handshake here and then forwards. */
	if (snap.state == ST_PENDING) {
		struct ent run;
		int err;

		if (!tbl_claim(sk, &run))
			return 1;			/* lost the claim race */
		err = do_handshake(&run, atomic_inc_return(&conn_seq));
		tbl_del(sk);
		return err < 0 ? err : 0;
	}
	/* ST_HANDSHAKING on a non-kd socket: a concurrent second app call while
	 * the first drives inline. Don't touch the socket. */
	return caller_nb ? -EAGAIN : wait_for_driver(sk);
}

static int hooked_sendmsg(struct sock *sk, struct msghdr *msg, size_t size)
{
	int g;

	if (tbl_state(sk) == ST_FREE)
		return orig_sendmsg(sk, msg, size);
	g = io_gate(sk, call_nonblocking(sk, msg->msg_flags));
	if (g == 1)
		return orig_sendmsg(sk, msg, size);
	if (g < 0)
		return g;
	return sk->sk_prot->sendmsg(sk, msg, size);	/* sk_prot is now tls */
}
static int hooked_recvmsg(struct sock *sk, struct msghdr *msg, size_t len, int flags, int *addr_len)
{
	int g;

	if (tbl_state(sk) == ST_FREE)
		return orig_recvmsg(sk, msg, len, flags, addr_len);
	g = io_gate(sk, call_nonblocking(sk, flags));
	if (g == 1)
		return orig_recvmsg(sk, msg, len, flags, addr_len);
	if (g < 0)
		return g;
	return sk->sk_prot->recvmsg(sk, msg, len, flags, addr_len);
}

static int hooked_connect(struct sock *sk, struct sockaddr_unsized *uaddr, int addr_len)
{
	struct sockaddr_in *sin = (struct sockaddr_in *)uaddr;

	tbl_del(sk);	/* evict any stale entry from a reused sk pointer */
	if (uaddr && addr_len >= (int)sizeof(*sin) && sin->sin_family == AF_INET &&
	    is_mesh(sin->sin_addr.s_addr, sin->sin_port)) {
		__be32 vip = sin->sin_addr.s_addr, bip = 0;
		__be16 vport = sin->sin_port, bport = 0;
		char peer[128] = "";
		struct ent *e;
		int err, ret, nb;

		err = do_resolve(sk, vip, vport, &bip, &bport, peer, sizeof(peer));
		if (err)
			return err;		/* DENY -> -EACCES (no TCP SYN); or resolve error */
		sin->sin_addr.s_addr = bip;	/* rewrite dst to the chosen backend */
		sin->sin_port = bport;
		nb = (sk->sk_socket && sk->sk_socket->file &&
		      (sk->sk_socket->file->f_flags & O_NONBLOCK)) ? 1 : 0;
		ret = orig_connect(sk, uaddr, addr_len);	/* nb: returns -EINPROGRESS */
		if (ret != 0 && ret != -EINPROGRESS)
			return ret;
		e = tbl_add(sk, ROLE_CLIENT);
		if (e) {
			unsigned long f;

			spin_lock_irqsave(&tbl_lock, f);
			e->ip = bip; e->port = bport; e->kd = nb;
			strscpy(e->peer, peer, sizeof(e->peer));
			spin_unlock_irqrestore(&tbl_lock, f);
		}
		if (nb && e) {
			/* Part 2 client: withhold readiness from this moment (so no
			 * spurious EPOLLOUT when TCP comes up) and drive the handshake
			 * in the background. */
			igkm_swap_ops(sk);
			spawn_driver(sk, "connect");
			pr_info(PFX "[connect] NON-BLOCKING mesh connect %pI4:%u -> EINPROGRESS; readiness withheld, handshake driven by kthread\n",
				&bip, ntohs(bport));
		}
		return ret;
	}
	return orig_connect(sk, uaddr, addr_len);
}

static struct sock *hooked_accept(struct sock *sk, struct proto_accept_arg *arg)
{
	struct sock *child = orig_accept(sk, arg);
	unsigned int lport;

	if (!child)
		return child;
	tbl_del(child);	/* evict any stale entry from a reused child sk pointer */
	lport = inet_sk(sk)->inet_num;	/* listener local port, host order */
	if (is_mesh_listen(lport)) {
		struct inet_sock *ci = inet_sk(child);
		struct ent *e;

		/* born-suppress every FUTURE child: do_accept copies newsock->ops
		 * from the listener's ops, so swapping the listener's ops means the
		 * next accepted child inherits our masking ->poll from birth. */
		igkm_swap_ops(sk);

		e = tbl_add(child, ROLE_SERVER);
		if (e) {
			unsigned long f;

			spin_lock_irqsave(&tbl_lock, f);
			e->kd = 1;		/* server handshake is always kthread-driven */
			e->lsaddr = ci->inet_rcv_saddr; e->lsport = htons(ci->inet_num);
			e->rdaddr = ci->inet_daddr;     e->rdport = ci->inet_dport;
			spin_unlock_irqrestore(&tbl_lock, f);
			pr_info(PFX "[accept] inbound on mesh listen :%u -> child local %pI4:%u remote %pI4:%u; server-side mTLS, handshake driven by kthread\n",
				lport, &e->lsaddr, ntohs(e->lsport), &e->rdaddr, ntohs(e->rdport));
			spawn_driver(child, "accept");
		}
	}
	return child;
}

static int __init igkme_init(void)
{
	struct proto_ops *isop;

	pr_info(PFX "init on stock kernel; mesh=\"%s\" mesh_listen=\"%s\" agent cid=%u port=%u\n",
		mesh ? mesh : "", mesh_listen ? mesh_listen : "", agent_cid, agent_port);
	if (resolve_kln()) { pr_err(PFX "kallsyms resolve failed\n"); return -ENOENT; }
	tcp_prot_p = (struct proto *)kln("tcp_prot");
	p_tcp_set_ulp = (void *)kln("tcp_set_ulp");
	isop = (struct proto_ops *)kln("inet_stream_ops");
	if (!tcp_prot_p || !p_tcp_set_ulp || !isop) { pr_err(PFX "symbol resolve failed\n"); return -ENOENT; }

	/* private proto_ops copy with our masking ->poll */
	memcpy(&igkm_stream_ops, isop, sizeof(igkm_stream_ops));
	orig_poll = isop->poll;
	igkm_stream_ops.poll = igkm_poll;

	orig_connect = tcp_prot_p->connect;
	orig_accept = tcp_prot_p->accept;
	orig_sendmsg = tcp_prot_p->sendmsg;
	orig_recvmsg = tcp_prot_p->recvmsg;
	WRITE_ONCE(tcp_prot_p->sendmsg, hooked_sendmsg);
	WRITE_ONCE(tcp_prot_p->recvmsg, hooked_recvmsg);
	WRITE_ONCE(tcp_prot_p->connect, hooked_connect);
	WRITE_ONCE(tcp_prot_p->accept, hooked_accept);
	pr_info(PFX "hooked tcp_prot connect/accept/sendmsg/recvmsg; private proto_ops.poll ready (orig_poll=%ps)\n",
		orig_poll);
	return 0;
}
static void __exit igkme_exit(void)
{
	int i;

	if (tcp_prot_p) {
		WRITE_ONCE(tcp_prot_p->connect, orig_connect);
		WRITE_ONCE(tcp_prot_p->accept, orig_accept);
		WRITE_ONCE(tcp_prot_p->sendmsg, orig_sendmsg);
		WRITE_ONCE(tcp_prot_p->recvmsg, orig_recvmsg);
	}
	/* best-effort: stop any still-running drivers (guest reboots after the
	 * run, so swapped per-socket ->ops are not a live rmmod concern here --
	 * noted as teardown debt, same class as the tcp_prot pointer swap). */
	for (i = 0; i < NENT; i++)
		if (tbl[i].kth) { kthread_stop(tbl[i].kth); tbl[i].kth = NULL; }
	synchronize_rcu();
	msleep(100);
	pr_info(PFX "unhooked; exit\n");
}
module_init(igkme_init);
module_exit(igkme_exit);
