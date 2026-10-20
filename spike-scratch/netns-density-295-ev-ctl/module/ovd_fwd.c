// SPDX-License-Identifier: GPL-2.0
/*
 * ev-ctl spike module: the PROPOSED EVMOD-CTL / EVMOD-LIFE control surface (docs/feature/netns-density-295/feature-delta.md,
 * "Control interface") grafted onto the evmod forwarding core (spike-scratch/netns-density-295-evmod/inc2/module/evmod.c).
 *
 *  - misc device /dev/ovd_fwd (0600); each open() is a context that owns its pairs and watches
 *  - context release aborts every live pair and watch before it returns and queues no record
 *  - ids: u64 from 1, never reused within a context
 *  - one preallocated 32-byte end record per id, read() / poll()
 *  - module_init only, no module_exit: delete_module returns -EBUSY (kernel/module/main.c:818-826 @ v7.0)
 *
 * SPIKE-ONLY: /proc/ovd_fwd_audit (global counters used as evidence), mode 1/2 (datagram) return -EOPNOTSUPP,
 * and the end classification is the minimum the control tests need (V-EV-END owns the full classification).
 */
#include <linux/module.h>
#include <linux/miscdevice.h>
#include <linux/fs.h>
#include <linux/net.h>
#include <linux/uio.h>
#include <linux/slab.h>
#include <linux/xarray.h>
#include <linux/refcount.h>
#include <linux/workqueue.h>
#include <linux/completion.h>
#include <linux/proc_fs.h>
#include <linux/seq_file.h>
#include <linux/poll.h>
#include <linux/vm_sockets.h>
#include <linux/rcupdate.h>
#include <net/sock.h>
#include <net/tcp.h>
#include <net/af_vsock.h>
#include "ovd_fwd.h"

#define FWD_CHUNK       65536U
#define FWD_RUN_BUDGET  (4U * FWD_CHUNK)
#define EV_POOL         256

static int strict_established;
module_param(strict_established, int, 0444);
MODULE_PARM_DESC(strict_established, "1 = a TCP half must be ESTABLISHED (the evmod spike rule); 0 = ESTABLISHED or CLOSE_WAIT");

#define EV_D0   1UL
#define EV_D1   2UL
#define EV_END  4UL

/* end codes (low byte of pair->end) */
#define EC_CLEAN    0x80
#define EC_PEER     1
#define EC_SOCKERR  2
#define EC_FRAME    3
#define EC_DETACHED 4

struct ev_ctx;
struct ev_pair;

struct ev_rec {
	struct list_head node;
	struct ovd_fwd_end e;
};

struct ev_half {
	struct socket *sock;
	struct sock *sk;
	struct ev_pair *pair;
	void (*orig_data_ready)(struct sock *);
	void (*orig_write_space)(struct sock *);
	void (*orig_state_change)(struct sock *);
	void (*orig_error_report)(struct sock *);
	int idx;
	bool tcp;
	bool hooked;
	bool watch_only;
	bool src_done;
	bool clean_eof;   /* set in the callback, under the socket lock */
	bool we_shut;     /* this forwarder shut the half's send direction */
	u32 ps_seen;      /* peer_shutdown bits already classified */
};

struct ev_pair {
	refcount_t ref;
	struct rcu_head rcu;
	struct work_struct work;
	atomic_long_t pending;
	bool armed;
	bool dead;
	bool watch;
	bool releasing;
	struct ev_ctx *ctx;
	u64 id;
	struct ev_half h[2];
	atomic_t end;
	int err_side;
	u64 moved[2];
	struct ev_rec *rec;
	struct completion torn;
};

struct ev_ctx {
	refcount_t ref;
	spinlock_t lock;            /* ends, dying, counters */
	struct xarray live;         /* id -> pair, present only for armed live ids */
	atomic64_t next_id;
	struct list_head ends;
	u32 n_ends;
	wait_queue_head_t wq;
	bool dying;
	u32 live_pairs, live_watches;
	atomic64_t st_attached, st_watched, st_completed, st_halfclosed;
	atomic64_t st_ab[5];
	atomic64_t st_unclassified;
};

static struct workqueue_struct *ev_wq;
static void *ev_pool[EV_POOL];
static int ev_pool_n;
static DEFINE_SPINLOCK(ev_pool_lock);

/* global audit counters (spike evidence only) */
static atomic64_t g_ctx_open, g_ctx_rel, g_pair_new, g_pair_free, g_rec_new, g_rec_queued, g_rec_read, g_rec_discard,
		  g_rec_free_unread, g_rel_won, g_live, g_ctx_free, g_bytes;

static void ev_put(struct ev_pair *p);

/* ---- records ------------------------------------------------------------------------------------------------------ */
static void ev_queue_rec(struct ev_pair *p)
{
	struct ev_ctx *c = p->ctx;
	struct ev_rec *r = xchg(&p->rec, NULL);
	u32 e = atomic_read(&p->end);
	u8 code = e & 0xff, side = (e >> 8) & 0xff;
	u16 err = e >> 16;
	bool clean = code == EC_CLEAN;

	if (!r)
		return;
	r->e.id = p->id;
	r->e.kind = p->watch ? (clean ? 3 : 4) : (clean ? 1 : 2);
	r->e.cause = clean ? 0 : code;
	r->e.side = (!clean && code >= EC_PEER && code <= EC_FRAME) ? side : 0;
	r->e.err = (!clean && code == EC_SOCKERR) ? err : 0;
	r->e.moved_ab = p->moved[0];
	r->e.moved_ba = p->moved[1];
	spin_lock_bh(&c->lock);
	if (c->dying) {
		spin_unlock_bh(&c->lock);
		atomic64_inc(&g_rec_discard);
		kfree(r);
		return;
	}
	list_add_tail(&r->node, &c->ends);
	c->n_ends++;
	spin_unlock_bh(&c->lock);
	atomic64_inc(&g_rec_queued);
	wake_up_interruptible_poll(&c->wq, EPOLLIN | EPOLLRDNORM);
}

/* ---- scheduling --------------------------------------------------------------------------------------------------- */
static void ev_kick(struct ev_pair *p, unsigned long mask)
{
	atomic_long_or(mask, &p->pending);
	smp_mb__after_atomic();
	if (!READ_ONCE(p->armed))
		return;
	if (!refcount_inc_not_zero(&p->ref))
		return;
	if (!queue_work(ev_wq, &p->work))
		ev_put(p);
}

/* First decision wins; the code is packed in one atomic so side and errno cannot be torn. */
static bool ev_decide(struct ev_pair *p, u8 code, u8 side, u16 err)
{
	struct ev_ctx *c = p->ctx;

	if (atomic_cmpxchg(&p->end, 0, (int)(code | (side << 8) | ((u32)err << 16))) != 0)
		return false;
	if (code == EC_CLEAN)
		atomic64_inc(p->watch ? &c->st_halfclosed : &c->st_completed);
	else
		atomic64_inc(&c->st_ab[code]);
	return true;
}

/* A decision to end. A watch's record is queued here, in the callback, so it precedes the holder's read of the end; a pair's
 * record is queued by the worker after the abort action, when its byte counters are final. */
static bool ev_end_req(struct ev_pair *p, u8 code, u8 side, u16 err)
{
	if (!ev_decide(p, code, side, err))
		return false;
	if (p->watch)
		ev_queue_rec(p);
	ev_kick(p, EV_END);
	return true;
}

/* ---- classification ----------------------------------------------------------------------------------------------- */
/* Called with the vsock socket locked (rx path, shutdown path) or lock_sock held (hook). */
static void ev_vsock_classify(struct ev_half *h)
{
	struct ev_pair *p = h->pair;
	u32 ps = READ_ONCE(vsock_sk(h->sk)->peer_shutdown) & SHUTDOWN_MASK;
	u32 nb = ps & ~h->ps_seen;

	if (!nb)
		return;
	h->ps_seen = ps;
	if (nb & SEND_SHUTDOWN) {
		if (ps & RCV_SHUTDOWN) {
			ev_end_req(p, EC_PEER, h->idx, 0);          /* 0x3 or RST with the peer's direction open */
		} else {
			WRITE_ONCE(h->clean_eof, true);              /* 0x2: clean half close of the peer's direction */
			if (p->watch)
				ev_end_req(p, EC_CLEAN, 0, 0);
			else
				ev_kick(p, h->idx ? EV_D1 : EV_D0);
		}
	} else if (!READ_ONCE(h->we_shut)) {
		ev_end_req(p, EC_PEER, h->idx, 0);                  /* the peer stopped receiving while we still send */
	}                                                           /* else: CLEAN-FULL-END, END-IDEMPOTENT: nothing */
}

/* ---- socket callbacks --------------------------------------------------------------------------------------------- */
static void ev_data_ready(struct sock *sk)
{
	struct ev_half *h;

	rcu_read_lock();
	h = rcu_dereference_sk_user_data(sk);
	if (h)
		ev_kick(h->pair, h->idx ? EV_D1 : EV_D0);
	rcu_read_unlock();
}

static void ev_write_space(struct sock *sk)
{
	struct ev_half *h;

	rcu_read_lock();
	h = rcu_dereference_sk_user_data(sk);
	if (h) {
		h->orig_write_space(sk);
		if (!h->tcp || sk_stream_is_writeable(sk))
			ev_kick(h->pair, h->idx ? EV_D0 : EV_D1);
	}
	rcu_read_unlock();
}

static void ev_state_change(struct sock *sk)
{
	struct ev_half *h;

	rcu_read_lock();
	h = rcu_dereference_sk_user_data(sk);
	if (h) {
		h->orig_state_change(sk);
		if (!h->tcp)
			ev_vsock_classify(h);
		ev_kick(h->pair, EV_D0 | EV_D1);
	}
	rcu_read_unlock();
}

static void ev_error_report(struct sock *sk)
{
	struct ev_half *h;

	rcu_read_lock();
	h = rcu_dereference_sk_user_data(sk);
	if (h) {
		int err = READ_ONCE(sk->sk_err);

		h->orig_error_report(sk);
		if (!h->tcp && err)
			ev_end_req(h->pair, EC_SOCKERR, h->idx, err);
		ev_kick(h->pair, EV_D0 | EV_D1);
	}
	rcu_read_unlock();
}

static int ev_hook(struct ev_half *h)
{
	struct sock *sk = h->sk;
	int ret = 0;

	lock_sock(sk);
	write_lock_bh(&sk->sk_callback_lock);
	if (sk->sk_user_data) {
		ret = -EBUSY;
		goto out;
	}
	h->orig_data_ready = sk->sk_data_ready;
	h->orig_write_space = sk->sk_write_space;
	h->orig_state_change = sk->sk_state_change;
	h->orig_error_report = sk->sk_error_report;
	rcu_assign_sk_user_data(sk, h);
	if (!h->watch_only) {
		WRITE_ONCE(sk->sk_data_ready, ev_data_ready);
		WRITE_ONCE(sk->sk_write_space, ev_write_space);
	}
	WRITE_ONCE(sk->sk_state_change, ev_state_change);
	WRITE_ONCE(sk->sk_error_report, ev_error_report);
	h->hooked = true;
out:
	write_unlock_bh(&sk->sk_callback_lock);
	if (!ret && !h->tcp)
		ev_vsock_classify(h);       /* state already reached before the hook */
	release_sock(sk);
	return ret;
}

/* Under lock_sock so no rx-path callback is mid-flight on the socket when the originals come back. */
static void ev_unhook(struct ev_half *h)
{
	struct sock *sk = h->sk;

	if (!h->hooked)
		return;
	lock_sock(sk);
	write_lock_bh(&sk->sk_callback_lock);
	if (!h->watch_only) {
		WRITE_ONCE(sk->sk_data_ready, h->orig_data_ready);
		WRITE_ONCE(sk->sk_write_space, h->orig_write_space);
	}
	WRITE_ONCE(sk->sk_state_change, h->orig_state_change);
	WRITE_ONCE(sk->sk_error_report, h->orig_error_report);
	rcu_assign_sk_user_data(sk, NULL);
	h->hooked = false;
	write_unlock_bh(&sk->sk_callback_lock);
	release_sock(sk);
}

/* ---- scratch pool ------------------------------------------------------------------------------------------------- */
static void *ev_buf_get(void)
{
	void *b = NULL;

	spin_lock_bh(&ev_pool_lock);
	if (ev_pool_n)
		b = ev_pool[--ev_pool_n];
	spin_unlock_bh(&ev_pool_lock);
	if (!b)
		b = kvmalloc(FWD_CHUNK, GFP_KERNEL);
	return b;
}

static void ev_buf_put(void *b)
{
	bool kept = false;

	spin_lock_bh(&ev_pool_lock);
	if (ev_pool_n < EV_POOL) {
		ev_pool[ev_pool_n++] = b;
		kept = true;
	}
	spin_unlock_bh(&ev_pool_lock);
	if (!kept)
		kvfree(b);
}

/* ---- forwarding --------------------------------------------------------------------------------------------------- */
enum { EV_BLOCKED = 0, EV_MORE = 1, EV_EOF = 2, EV_STOP = 3 };

static int ev_consume(struct socket *s, size_t n)
{
	struct msghdr m = { .msg_flags = MSG_DONTWAIT };
	int r;

	iov_iter_discard(&m.msg_iter, ITER_DEST, n);
	r = sock_recvmsg(s, &m, MSG_DONTWAIT);
	if (r < 0)
		return r;
	return (size_t)r == n ? 0 : -EIO;
}

static int ev_send(struct socket *t, void *buf, int n)
{
	struct msghdr m = { .msg_flags = MSG_DONTWAIT | MSG_NOSIGNAL };
	struct kvec v = { .iov_base = buf, .iov_len = n };

	return kernel_sendmsg(t, &m, &v, 1, n);
}

static int ev_fwd(struct ev_pair *p, int d, void *buf, int *budget)
{
	struct ev_half *s = &p->h[d], *t = &p->h[!d];

	if (s->src_done)
		return EV_BLOCKED;
	while (*budget > 0) {
		struct msghdr rm = { .msg_flags = MSG_DONTWAIT };
		struct kvec rv = { .iov_base = buf, .iov_len = FWD_CHUNK };
		int n, sent;

		if (atomic_read(&p->end))
			return EV_STOP;              /* never forward a byte past the first end decision */
		n = kernel_recvmsg(s->sock, &rm, &rv, 1, FWD_CHUNK, MSG_DONTWAIT | MSG_PEEK);
		if (n == -EAGAIN)
			return EV_BLOCKED;
		if (n < 0) {
			p->err_side = d;
			return n;
		}
		if (n == 0) {
			if (!s->tcp && !READ_ONCE(s->clean_eof)) {
				atomic64_inc(&p->ctx->st_unclassified);
				ev_end_req(p, EC_PEER, d, 0);
				return EV_STOP;
			}
			s->src_done = true;
			WRITE_ONCE(t->we_shut, true);
			kernel_sock_shutdown(t->sock, SHUT_WR);
			return EV_EOF;
		}
		sent = ev_send(t->sock, buf, n);
		if (sent == -EAGAIN)
			return EV_BLOCKED;
		if (sent < 0) {
			p->err_side = !d;
			return sent;
		}
		if (sent > 0) {
			int r = ev_consume(s->sock, sent);

			if (r) {
				p->err_side = d;
				return r;
			}
			p->moved[d] += sent;
			atomic64_add(sent, &g_bytes);
			*budget -= sent;
		}
		if (sent < n)
			return EV_BLOCKED;
	}
	return EV_MORE;
}

static void ev_finish(struct ev_pair *p)
{
	struct ev_ctx *c = p->ctx;
	u8 code = atomic_read(&p->end) & 0xff;
	bool act = code != EC_CLEAN;
	int i, n = p->watch ? 1 : 2;

	/* A watch ended by the owner's ABORT only stops watching; the holder owns the socket. Release resets it. */
	if (p->watch && code == EC_DETACHED && !p->releasing)
		act = false;
	p->dead = true;
	for (i = 0; i < n; i++)
		ev_unhook(&p->h[i]);
	if (act) {
		for (i = 0; i < n; i++) {
			if (p->h[i].tcp)
				tcp_abort(p->h[i].sk, ECONNABORTED);   /* sends a reset on a live TCP half; -ENOENT if already closed */
			else
				kernel_sock_shutdown(p->h[i].sock, SHUT_RDWR);
		}
	}
	xa_erase(&c->live, p->id);
	spin_lock_bh(&c->lock);
	if (p->watch)
		c->live_watches--;
	else
		c->live_pairs--;
	spin_unlock_bh(&c->lock);
	atomic64_dec(&g_live);
	if (!p->watch)
		ev_queue_rec(p);
	kfree(xchg(&p->rec, NULL));
	for (i = 0; i < n; i++) {
		if (p->h[i].sock) {
			sockfd_put(p->h[i].sock);
			p->h[i].sock = NULL;
		}
	}
	complete_all(&p->torn);
	ev_put(p);   /* the xarray's reference; p->ctx is not touched again */
}

static void ev_work(struct work_struct *w)
{
	struct ev_pair *p = container_of(w, struct ev_pair, work);
	int budget[2] = { FWD_RUN_BUDGET, FWD_RUN_BUDGET };
	unsigned long again = 0, m;
	void *buf = NULL;
	int err = 0, d;

	if (p->dead)
		goto out;
	if (p->watch) {
		atomic_long_xchg(&p->pending, 0);
		if (atomic_read(&p->end))
			ev_finish(p);
		goto out;
	}
	while ((m = atomic_long_xchg(&p->pending, 0)) != 0) {
		if (atomic_read(&p->end)) {
			ev_finish(p);
			goto out;
		}
		if (!buf) {
			buf = ev_buf_get();
			if (!buf) {
				err = -ENOMEM;
				break;
			}
		}
		for (d = 0; d < 2 && !err; d++) {
			int r;

			if (!(m & (d ? EV_D1 : EV_D0)))
				continue;
			if (budget[d] <= 0) {
				again |= d ? EV_D1 : EV_D0;
				continue;
			}
			r = ev_fwd(p, d, buf, &budget[d]);
			if (r < 0)
				err = r;
			else if (r == EV_MORE)
				again |= d ? EV_D1 : EV_D0;
		}
		if (err || again)
			break;
	}
	if (err) {
		ev_end_req(p, EC_SOCKERR, p->err_side, -err);
	} else if (p->h[0].src_done && p->h[1].src_done) {
		ev_decide(p, EC_CLEAN, 0, 0);
	}
	if (atomic_read(&p->end)) {
		ev_finish(p);
		goto out;
	}
	if (again)
		ev_kick(p, again);
out:
	if (buf)
		ev_buf_put(buf);
	ev_put(p);
}

/* ---- pair lifetime ------------------------------------------------------------------------------------------------ */
static void ev_ctx_put(struct ev_ctx *c)
{
	if (refcount_dec_and_test(&c->ref)) {
		atomic64_inc(&g_ctx_free);
		xa_destroy(&c->live);
		kfree(c);
	}
}

/* The pair, and the reference it holds on its context, outlive every callback that may still be inside rcu_read_lock(). */
static void ev_free_rcu(struct rcu_head *rcu)
{
	struct ev_pair *p = container_of(rcu, struct ev_pair, rcu);
	struct ev_ctx *c = p->ctx;

	atomic64_inc(&g_pair_free);
	kfree(p);
	ev_ctx_put(c);
}

static void ev_put(struct ev_pair *p)
{
	if (refcount_dec_and_test(&p->ref))
		call_rcu(&p->rcu, ev_free_rcu);
}

static struct ev_pair *ev_pair_new(struct ev_ctx *c, bool watch)
{
	struct ev_pair *p = kzalloc(sizeof(*p), GFP_KERNEL);

	if (!p)
		return NULL;
	p->rec = kzalloc(sizeof(*p->rec), GFP_KERNEL);
	if (!p->rec) {
		kfree(p);
		return NULL;
	}
	atomic64_inc(&g_rec_new);
	atomic64_inc(&g_pair_new);
	refcount_set(&p->ref, 1);
	refcount_inc(&c->ref);
	p->ctx = c;
	p->watch = watch;
	INIT_WORK(&p->work, ev_work);
	init_completion(&p->torn);
	p->id = (u64)atomic64_inc_return(&c->next_id);   /* consumed even if the attach then fails: ids are never reused */
	return p;
}

/* a pair that never became live: nothing was queued, nobody else holds it */
static void ev_pair_discard(struct ev_pair *p)
{
	int i;

	for (i = 0; i < 2; i++) {
		ev_unhook(&p->h[i]);
		if (p->h[i].sock)
			sockfd_put(p->h[i].sock);
	}
	kfree(xchg(&p->rec, NULL));
	call_rcu(&p->rcu, ev_free_rcu);
}

/* ---- attach / watch ----------------------------------------------------------------------------------------------- */
enum { K_OTHER, K_VSOCK, K_TCP };

static int ev_kind(struct socket *s)
{
	struct sock *sk = s->sk;

	if (sk->sk_family == AF_VSOCK && s->type == SOCK_STREAM)
		return K_VSOCK;
	if ((sk->sk_family == AF_INET || sk->sk_family == AF_INET6) && s->type == SOCK_STREAM && sk->sk_protocol == IPPROTO_TCP)
		return K_TCP;
	return K_OTHER;
}

static bool ev_state_ok(struct socket *s, int kind)
{
	int st = READ_ONCE(s->sk->sk_state);

	if (kind == K_TCP && !strict_established)
		return st == TCP_ESTABLISHED || st == TCP_CLOSE_WAIT;   /* CLOSE_WAIT: the peer's FIN arrived before the attach */
	return st == TCP_ESTABLISHED;
}

static void ev_half_set(struct ev_half *h, struct ev_pair *p, int idx, struct socket *s, int kind)
{
	h->sock = s;
	h->sk = s->sk;
	h->pair = p;
	h->idx = idx;
	h->tcp = kind == K_TCP;
}

static void ev_register(struct ev_ctx *c, struct ev_pair *p)
{
	void *old;

	old = xa_store(&c->live, p->id, p, GFP_ATOMIC);   /* the slot was reserved: cannot fail */
	WARN_ON_ONCE(xa_is_err(old));
	spin_lock_bh(&c->lock);
	if (p->watch)
		c->live_watches++;
	else
		c->live_pairs++;
	spin_unlock_bh(&c->lock);
	atomic64_inc(&g_live);
	atomic64_inc(p->watch ? &c->st_watched : &c->st_attached);
	WRITE_ONCE(p->armed, true);
	smp_mb();
	if (p->watch) {
		if (atomic_read(&p->end))
			ev_kick(p, EV_END);
	} else {
		ev_kick(p, EV_D0 | EV_D1);   /* bytes or an end already queued on either half are forwarded by this first run */
	}
}

static long ev_attach(struct ev_ctx *c, struct ovd_fwd_attach __user *u)
{
	struct ovd_fwd_attach a;
	struct socket *sa = NULL, *sb = NULL;
	struct ev_pair *p;
	int ka, kb, err = 0;
	u8 bad = 0;

	if (copy_from_user(&a, u, sizeof(a)))
		return -EFAULT;
	if (a.flags || a.mode > OVD_FWD_MODE_DGRAM_UDP_B) {
		err = -EINVAL;
		goto out;
	}
	sa = sockfd_lookup(a.fd_a, &err);
	if (!sa) {
		bad = 0;
		goto out;
	}
	sb = sockfd_lookup(a.fd_b, &err);
	if (!sb) {
		bad = 1;
		goto out_put;
	}
	if (sa->sk == sb->sk) {
		bad = 1;
		err = -EINVAL;
		goto out_put;
	}
	if (a.mode != OVD_FWD_MODE_STREAM) {
		err = -EOPNOTSUPP;   /* SPIKE: datagram mode is V-EV-DGRAM's */
		goto out_put;
	}
	ka = ev_kind(sa);
	kb = ev_kind(sb);
	if (ka == K_OTHER || (ka == kb)) {
		bad = ka == K_OTHER ? 0 : 1;
		err = -ESOCKTNOSUPPORT;
		goto out_put;
	}
	if (kb == K_OTHER) {
		bad = 1;
		err = -ESOCKTNOSUPPORT;
		goto out_put;
	}
	if (ka == K_TCP && inet_csk_has_ulp(sa->sk)) {
		bad = 0;
		err = -EPROTONOSUPPORT;
		goto out_put;
	}
	if (kb == K_TCP && inet_csk_has_ulp(sb->sk)) {
		bad = 1;
		err = -EPROTONOSUPPORT;
		goto out_put;
	}
	if (!ev_state_ok(sa, ka)) {
		bad = 0;
		err = -ENOTCONN;
		goto out_put;
	}
	if (!ev_state_ok(sb, kb)) {
		bad = 1;
		err = -ENOTCONN;
		goto out_put;
	}
	p = ev_pair_new(c, false);
	if (!p) {
		err = -ENOMEM;
		goto out_put;
	}
	ev_half_set(&p->h[0], p, 0, sa, ka);
	ev_half_set(&p->h[1], p, 1, sb, kb);
	sa = sb = NULL;              /* the pair owns the file references now */
	err = xa_reserve(&c->live, p->id, GFP_KERNEL);
	if (err) {
		ev_pair_discard(p);
		goto out;
	}
	err = ev_hook(&p->h[0]);
	if (err) {
		bad = 0;
		xa_release(&c->live, p->id);
		ev_pair_discard(p);
		goto out;
	}
	err = ev_hook(&p->h[1]);
	if (err) {
		bad = 1;
		xa_release(&c->live, p->id);
		ev_pair_discard(p);
		goto out;
	}
	a.id = p->id;
	ev_register(c, p);
	err = 0;
	goto out;
out_put:
	if (sa)
		sockfd_put(sa);
	if (sb)
		sockfd_put(sb);
out:
	a.bad_side = err ? bad : 0;
	if (copy_to_user(u, &a, sizeof(a)) && !err)
		return -EFAULT;   /* the pair is live and its id unreported: the owner's close of the context ends it */
	return err;
}

static long ev_watch(struct ev_ctx *c, struct ovd_fwd_watch __user *u)
{
	struct ovd_fwd_watch w;
	struct socket *s;
	struct ev_pair *p;
	int err = 0;

	if (copy_from_user(&w, u, sizeof(w)))
		return -EFAULT;
	if (w.flags)
		return -EINVAL;
	s = sockfd_lookup(w.fd, &err);
	if (!s)
		return err;
	if (ev_kind(s) != K_VSOCK) {
		sockfd_put(s);
		return -ESOCKTNOSUPPORT;
	}
	if (!ev_state_ok(s, K_VSOCK)) {
		sockfd_put(s);
		return -ENOTCONN;
	}
	p = ev_pair_new(c, true);
	if (!p) {
		sockfd_put(s);
		return -ENOMEM;
	}
	ev_half_set(&p->h[0], p, 0, s, K_VSOCK);
	p->h[0].watch_only = true;
	err = xa_reserve(&c->live, p->id, GFP_KERNEL);
	if (err) {
		ev_pair_discard(p);
		return err;
	}
	err = ev_hook(&p->h[0]);
	if (err) {
		xa_release(&c->live, p->id);
		ev_pair_discard(p);
		return err;
	}
	w.id = p->id;
	w.bad_side = 0;
	ev_register(c, p);
	if (copy_to_user(u, &w, sizeof(w)))
		return -EFAULT;
	return 0;
}

/* ---- operations on ids -------------------------------------------------------------------------------------------- */
static struct ev_pair *ev_lookup(struct ev_ctx *c, u64 id)
{
	struct ev_pair *p;

	xa_lock(&c->live);
	p = xa_load(&c->live, id);
	if (p && !refcount_inc_not_zero(&p->ref))
		p = NULL;
	xa_unlock(&c->live);
	return p;
}

static long ev_abort(struct ev_ctx *c, u64 id)
{
	struct ev_pair *p = ev_lookup(c, id);
	bool won;

	if (!p)
		return -ENOENT;
	won = ev_end_req(p, EC_DETACHED, 0, 0);
	wait_for_completion(&p->torn);      /* the record is queued before ABORT returns, whether this call or an earlier end won */
	ev_put(p);
	return won ? 0 : -ENOENT;
}

static u32 ev_end_all(struct ev_ctx *c, bool release)
{
	struct ev_pair *batch[64];
	unsigned long idx = 0;
	u32 won = 0;

	for (;;) {
		int n = 0, i;

		xa_lock(&c->live);
		while (n < 64) {
			struct ev_pair *p = xa_find(&c->live, &idx, ULONG_MAX, XA_PRESENT);

			if (!p)
				break;
			if (refcount_inc_not_zero(&p->ref))
				batch[n++] = p;
			idx++;
		}
		xa_unlock(&c->live);
		if (!n)
			break;
		for (i = 0; i < n; i++) {
			if (release)
				batch[i]->releasing = true;
			if (ev_end_req(batch[i], EC_DETACHED, 0, 0))
				won++;
		}
		for (i = 0; i < n; i++) {
			wait_for_completion(&batch[i]->torn);
			ev_put(batch[i]);
		}
	}
	return won;
}

static long ev_inventory(struct ev_ctx *c, struct ovd_fwd_inventory __user *u)
{
	struct ovd_fwd_inventory v = {};

	spin_lock_bh(&c->lock);
	v.live_pairs = c->live_pairs;
	v.live_watches = c->live_watches;
	v.queued_ends = c->n_ends;
	spin_unlock_bh(&c->lock);
	v.attached = atomic64_read(&c->st_attached);
	v.watched = atomic64_read(&c->st_watched);
	v.completed = atomic64_read(&c->st_completed);
	v.half_closed = atomic64_read(&c->st_halfclosed);
	v.aborted_peer = atomic64_read(&c->st_ab[EC_PEER]);
	v.aborted_sockerr = atomic64_read(&c->st_ab[EC_SOCKERR]);
	v.aborted_frame = atomic64_read(&c->st_ab[EC_FRAME]);
	v.aborted_detached = atomic64_read(&c->st_ab[EC_DETACHED]);
	v.unclassified_eof = atomic64_read(&c->st_unclassified);
	return copy_to_user(u, &v, sizeof(v)) ? -EFAULT : 0;
}

static long ev_pair_state(struct ev_ctx *c, struct ovd_fwd_pair_state __user *u)
{
	struct ovd_fwd_pair_state s;
	struct ev_pair *p;
	u8 code;
	int d;

	if (copy_from_user(&s, u, sizeof(s)))
		return -EFAULT;
	p = ev_lookup(c, s.id);
	if (!p || p->watch) {
		if (p)
			ev_put(p);
		return -ENOENT;
	}
	code = atomic_read(&p->end) & 0xff;
	for (d = 0; d < 2; d++) {
		s.source_end[d] = p->h[d].src_done ? 1 : (code && code != EC_CLEAN ? 2 : 0);
		s.target_shut[d] = READ_ONCE(p->h[!d].we_shut);
		s.moved[d] = READ_ONCE(p->moved[d]);
	}
	ev_put(p);
	return copy_to_user(u, &s, sizeof(s)) ? -EFAULT : 0;
}

/* ---- the context file --------------------------------------------------------------------------------------------- */
static int ev_open(struct inode *inode, struct file *f)
{
	struct ev_ctx *c = kzalloc(sizeof(*c), GFP_KERNEL);

	if (!c)
		return -ENOMEM;
	refcount_set(&c->ref, 1);
	spin_lock_init(&c->lock);
	xa_init(&c->live);
	INIT_LIST_HEAD(&c->ends);
	init_waitqueue_head(&c->wq);
	f->private_data = c;
	atomic64_inc(&g_ctx_open);
	return nonseekable_open(inode, f);
}

static int ev_release(struct inode *inode, struct file *f)
{
	struct ev_ctx *c = f->private_data;
	struct ev_rec *r, *tmp;
	LIST_HEAD(drop);

	spin_lock_bh(&c->lock);
	c->dying = true;                      /* from here no record is queued */
	list_splice_init(&c->ends, &drop);
	c->n_ends = 0;
	spin_unlock_bh(&c->lock);
	list_for_each_entry_safe(r, tmp, &drop, node) {
		list_del(&r->node);
		atomic64_inc(&g_rec_free_unread);
		kfree(r);
	}
	atomic64_add(ev_end_all(c, true), &g_rel_won);   /* every live pair and watch is ended before release returns */
	atomic64_inc(&g_ctx_rel);
	ev_ctx_put(c);
	return 0;
}

static ssize_t ev_read(struct file *f, char __user *ubuf, size_t len, loff_t *off)
{
	struct ev_ctx *c = f->private_data;
	struct ev_rec *r, *tmp, *q;
	LIST_HEAD(got);
	size_t want, done = 0;
	int ret;

	if (len < sizeof(struct ovd_fwd_end))
		return -EINVAL;
	want = min_t(size_t, len / sizeof(struct ovd_fwd_end), 8192);
	for (;;) {
		size_t k = 0;

		spin_lock_bh(&c->lock);
		list_for_each_entry_safe(r, tmp, &c->ends, node) {
			if (k == want)
				break;
			list_move_tail(&r->node, &got);
			k++;
		}
		c->n_ends -= k;
		spin_unlock_bh(&c->lock);
		if (k)
			break;
		if (f->f_flags & O_NONBLOCK)
			return -EAGAIN;
		ret = wait_event_interruptible(c->wq, READ_ONCE(c->n_ends) != 0);
		if (ret)
			return ret;
	}
	list_for_each_entry_safe(r, tmp, &got, node) {
		if (copy_to_user(ubuf + done, &r->e, sizeof(r->e))) {
			size_t back = 0;

			list_for_each_entry(q, &got, node)
				back++;
			spin_lock_bh(&c->lock);
			list_splice(&got, &c->ends);   /* the unread records go back to the head, none lost */
			c->n_ends += back;
			spin_unlock_bh(&c->lock);
			return done ? (ssize_t)done : -EFAULT;
		}
		done += sizeof(r->e);
		atomic64_inc(&g_rec_read);
		list_del(&r->node);
		kfree(r);
	}
	return done;
}

static __poll_t ev_poll(struct file *f, poll_table *wait)
{
	struct ev_ctx *c = f->private_data;

	poll_wait(f, &c->wq, wait);
	return READ_ONCE(c->n_ends) ? EPOLLIN | EPOLLRDNORM : 0;
}

static long ev_ioctl(struct file *f, unsigned int cmd, unsigned long arg)
{
	struct ev_ctx *c = f->private_data;
	void __user *u = (void __user *)arg;

	switch (cmd) {
	case OVD_FWD_VERSION: {
		struct ovd_fwd_version v = { .abi_version = OVD_FWD_ABI_VERSION, .flags = OVD_FWD_FLAG_PERMANENT };

		return copy_to_user(u, &v, sizeof(v)) ? -EFAULT : 0;
	}
	case OVD_FWD_ATTACH:
		return ev_attach(c, u);
	case OVD_FWD_WATCH:
		return ev_watch(c, u);
	case OVD_FWD_ABORT: {
		struct ovd_fwd_id i;

		if (copy_from_user(&i, u, sizeof(i)))
			return -EFAULT;
		return ev_abort(c, i.id);
	}
	case OVD_FWD_ABORT_ALL: {
		struct ovd_fwd_count n = { .n = ev_end_all(c, false) };

		return copy_to_user(u, &n, sizeof(n)) ? -EFAULT : 0;
	}
	case OVD_FWD_INVENTORY:
		return ev_inventory(c, u);
	case OVD_FWD_PAIR_STATE:
		return ev_pair_state(c, u);
	}
	return -ENOTTY;
}

static const struct file_operations ev_fops = {
	.owner = THIS_MODULE,
	.open = ev_open,
	.release = ev_release,
	.read = ev_read,
	.poll = ev_poll,
	.unlocked_ioctl = ev_ioctl,
	.compat_ioctl = ev_ioctl,
};
static struct miscdevice ev_misc = { .minor = MISC_DYNAMIC_MINOR, .name = "ovd_fwd", .fops = &ev_fops, .mode = 0600 };

static int ev_audit(struct seq_file *m, void *v)
{
	seq_printf(m,
		   "ctx_open=%lld ctx_rel=%lld ctx_free=%lld pair_new=%lld pair_free=%lld rec_new=%lld rec_queued=%lld rec_read=%lld "
		   "rec_discard=%lld rec_free_unread=%lld rel_won=%lld live=%lld bytes=%lld\n",
		   atomic64_read(&g_ctx_open), atomic64_read(&g_ctx_rel), atomic64_read(&g_ctx_free), atomic64_read(&g_pair_new),
		   atomic64_read(&g_pair_free), atomic64_read(&g_rec_new), atomic64_read(&g_rec_queued), atomic64_read(&g_rec_read),
		   atomic64_read(&g_rec_discard), atomic64_read(&g_rec_free_unread), atomic64_read(&g_rel_won), atomic64_read(&g_live),
		   atomic64_read(&g_bytes));
	return 0;
}

static int __init ev_init(void)
{
	int i, rc;

	BUILD_BUG_ON(sizeof(struct ovd_fwd_end) != 32);
	BUILD_BUG_ON(offsetof(struct ovd_fwd_end, moved_ab) != 16);
	ev_wq = alloc_workqueue("ovd_fwd", WQ_PERCPU, 0);
	if (!ev_wq)
		return -ENOMEM;
	for (i = 0; i < 8; i++) {
		void *b = kvmalloc(FWD_CHUNK, GFP_KERNEL);

		if (!b) {
			rc = -ENOMEM;
			goto fail;
		}
		ev_pool[ev_pool_n++] = b;
	}
	rc = misc_register(&ev_misc);
	if (rc)
		goto fail;
	proc_create_single("ovd_fwd_audit", 0444, NULL, ev_audit);
	pr_info("ovd_fwd: loaded (spike, permanent: no module_exit)\n");
	return 0;
fail:
	while (ev_pool_n)
		kvfree(ev_pool[--ev_pool_n]);
	destroy_workqueue(ev_wq);
	return rc;
}

module_init(ev_init);   /* no module_exit: delete_module returns -EBUSY (kernel/module/main.c:818-826 @ v7.0) */
MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("ev-ctl spike: EVMOD-CTL control surface over the evmod forwarding core");
