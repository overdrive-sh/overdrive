// SPDX-License-Identifier: GPL-2.0
/*
 * ovd_fwd (ev-end spike): the PROPOSED EVMOD contract, stream mode and watch only.
 *   - /dev/ovd_fwd, each open() is a context owning its pairs and watches; release aborts them all, queues no record.
 *   - ids are u64 from 1, never reused within a context; each id yields exactly one 32-byte end record, preallocated
 *     with the pair; read() returns whole records, poll() signals them.
 *   - module_init and NO module_exit (delete_module returns -EBUSY).
 *   - EVMOD-END: the class of a vsock end is computed per received SHUTDOWN / RST in the half's sk_state_change (and
 *     sk_err in sk_error_report), before the end is readable.
 * Derived from the evmod spike module (forwarding core). Spike code, not production.
 *
 * Test-only module parameters (hazard variants and stress, default off):
 *   hazard bit 0 (1): forward every vsock end-of-stream as a FIN, no classification (rejected alternative b)
 *   hazard bit 1 (2): classify at the EOF read from the accumulated peer_shutdown (not per transition)
 *   hazard bit 2 (4): a watch's record is queued from the work item, not the callback
 *   hazard bit 3 (8): a watch calls the original callback BEFORE queueing the record
 *   hazard bit 4 (16): a TCP half's sk_err is not read in sk_error_report (a reset is found only by recvmsg/sendmsg)
 *   work_delay_us: delay every pair work run, so callbacks run ahead of the forwarder
 */
#include <linux/module.h>
#include <linux/miscdevice.h>
#include <linux/fs.h>
#include <linux/poll.h>
#include <linux/net.h>
#include <linux/uio.h>
#include <linux/slab.h>
#include <linux/refcount.h>
#include <linux/workqueue.h>
#include <linux/completion.h>
#include <linux/rcupdate.h>
#include <linux/delay.h>
#include <linux/vm_sockets.h>
#include <net/sock.h>
#include <net/tcp.h>
#include <net/af_vsock.h>
#include "ovd_fwd.h"

static unsigned int hazard;
module_param(hazard, uint, 0644);
static unsigned int work_delay_us;
module_param(work_delay_us, uint, 0644);

#define OVD_CHUNK   65536U
#define OVD_BUDGET  (4U * OVD_CHUNK)

#define EV_D0       1UL
#define EV_D1       2UL
#define EV_TEARDOWN 4UL

enum { K_COMPLETED = 1, K_ABORTED = 2, K_W_HALFCLOSE = 3, K_W_ABORTED = 4 };
enum { C_NONE = 0, C_PEER = 1, C_SOCKET = 2, C_FRAME = 3, C_DETACHED = 4 };
enum { SE_NONE = 0, SE_CLEAN = 1, SE_ABORT = 2 };

struct ovd_ctx;
struct ovd_item;

struct ovd_half {
	struct socket *sock;
	struct sock *sk;
	struct ovd_item *it;
	void (*o_data_ready)(struct sock *);
	void (*o_write_space)(struct sock *);
	void (*o_state_change)(struct sock *);
	void (*o_error_report)(struct sock *);
	int idx;
	bool vsock;
	bool hooked;
	u8 acted;       /* vsock: peer_shutdown bits already classified */
	bool vclean;    /* vsock: the peer's direction was classified a clean half-close */
};

struct ovd_item {
	refcount_t ref;
	struct rcu_head rcu;
	struct ovd_ctx *ctx;
	u64 id;
	bool watch;
	struct delayed_work work;
	atomic_long_t pending;
	struct ovd_half h[2];
	int nh;
	spinlock_t lock;                  /* protects decision fields, acted bits, record */
	bool decided;
	u8 d_kind, d_cause, d_side;
	u16 d_err;
	bool d_norecord;
	bool d_noact;                     /* attach failure: no socket actions */
	bool recorded;                    /* record queued (watch: in the callback) */
	bool done;                        /* teardown executed */
	u8 src_end[2];
	bool tgt_shut[2];
	u64 moved[2];
	struct ovd_fwd_end rec;
	struct list_head live_node;
	struct list_head q_node;
	struct completion torn;
};

struct ovd_ctx {
	spinlock_t lock;
	struct list_head live;
	struct list_head ends;
	u32 n_pairs, n_watches, n_ends;
	u64 next_id;
	bool released;
	atomic_t nitems;
	wait_queue_head_t wq;
	atomic64_t attached, completed, aborted, unclassified_eof, ab_peer, ab_socket, ab_frame, ab_detached;
};

static struct workqueue_struct *ev_wq;

static void ev_put(struct ovd_item *it)
{
	if (refcount_dec_and_test(&it->ref))
		kfree_rcu(it, rcu);
}

static void ev_kick(struct ovd_item *it, unsigned long mask)
{
	atomic_long_or(mask, &it->pending);
	if (!refcount_inc_not_zero(&it->ref))
		return;
	if (!queue_delayed_work(ev_wq, &it->work, usecs_to_jiffies(work_delay_us)))
		ev_put(it);
}

/* ---- decision and record ------------------------------------------------------------ */
static bool ev_decide(struct ovd_item *it, u8 kind, u8 cause, u8 side, u16 err, bool norecord, bool noact)
{
	spin_lock_bh(&it->lock);
	if (it->decided) {
		spin_unlock_bh(&it->lock);
		return false;
	}
	it->decided = true;
	it->d_kind = kind;
	it->d_cause = cause;
	it->d_side = side;
	it->d_err = err;
	it->d_norecord = norecord;
	it->d_noact = noact;
	spin_unlock_bh(&it->lock);
	return true;
}

/* Fill the preallocated record and queue it on the context. Callable from the callback (BH/softirq) or the work item. */
static void ev_queue_record(struct ovd_item *it)
{
	struct ovd_ctx *ctx = it->ctx;

	spin_lock_bh(&it->lock);
	if (it->recorded || it->d_norecord) {
		spin_unlock_bh(&it->lock);
		return;
	}
	it->recorded = true;
	memset(&it->rec, 0, sizeof(it->rec));
	it->rec.id = it->id;
	it->rec.kind = it->d_kind;
	it->rec.cause = it->d_cause;
	it->rec.side = it->d_side;
	it->rec.err = it->d_err;
	it->rec.moved_ab = it->moved[0];
	it->rec.moved_ba = it->moved[1];
	spin_unlock_bh(&it->lock);

	spin_lock_bh(&ctx->lock);
	if (!ctx->released) {
		list_del_init(&it->live_node);
		if (it->watch)
			ctx->n_watches--;
		else
			ctx->n_pairs--;
		refcount_inc(&it->ref);   /* held while the record sits on the queue */
		list_add_tail(&it->q_node, &ctx->ends);
		ctx->n_ends++;
	}
	spin_unlock_bh(&ctx->lock);
	wake_up_interruptible(&ctx->wq);
}

static void ev_count_end(struct ovd_item *it)
{
	struct ovd_ctx *ctx = it->ctx;

	if (it->d_norecord)
		return;
	if (it->d_kind == K_COMPLETED) {
		atomic64_inc(&ctx->completed);
	} else if (it->d_kind == K_ABORTED) {
		atomic64_inc(&ctx->aborted);
		switch (it->d_cause) {
		case C_PEER: atomic64_inc(&ctx->ab_peer); break;
		case C_SOCKET: atomic64_inc(&ctx->ab_socket); break;
		case C_FRAME: atomic64_inc(&ctx->ab_frame); break;
		case C_DETACHED: atomic64_inc(&ctx->ab_detached); break;
		}
	}
}

/* ---- the classification (EVMOD-END), vsock half; lock held by the caller's path except the orphan sweep ------------ */
static void ev_classify_vsock(struct ovd_half *h, struct sock *sk)
{
	struct ovd_item *it = h->it;
	u8 ps = vsock_sk(sk)->peer_shutdown & SHUTDOWN_MASK;
	u8 newb;
	bool abort = false, own_closed;

	if (hazard & 1)
		goto kick;           /* hazard: no classification, every end-of-stream is forwarded as a FIN */
	spin_lock_bh(&it->lock);
	if (it->decided) {
		spin_unlock_bh(&it->lock);
		return;
	}
	newb = ps & ~h->acted;
	h->acted |= ps;
	own_closed = !!(sk->sk_shutdown & SEND_SHUTDOWN);
	if (!newb) {
		spin_unlock_bh(&it->lock);
		return;
	}
	if (hazard & 2) {
		/* hazard: no per-transition class; EOF-read time classifies from the accumulated state */
		spin_unlock_bh(&it->lock);
		goto kick;
	}
	if (newb & SEND_SHUTDOWN) {
		if (newb & RCV_SHUTDOWN)
			abort = true;                /* 0x3 or RST: the peer closed with its direction open */
		else
			h->vclean = true;            /* 0x2: clean half-close of the peer's direction */
	} else if (newb & RCV_SHUTDOWN) {
		if (!own_closed)
			abort = true;                /* the peer stopped receiving while this side still sends */
		/* else: the peer released a flow ended both ways, or read-shut after our last byte: END-IDEMPOTENT */
	}
	spin_unlock_bh(&it->lock);
	if (abort) {
		if (it->watch)
			goto watch_abort;
		if (ev_decide(it, K_ABORTED, C_PEER, h->idx, 0, false, false))
			ev_kick(it, EV_TEARDOWN);
		return;
	}
	if (h->vclean && it->watch) {
		if (ev_decide(it, K_W_HALFCLOSE, C_NONE, 0, 0, false, false)) {
			if (hazard & 4)
				ev_kick(it, EV_TEARDOWN);
			else {
				ev_queue_record(it);
				ev_kick(it, EV_TEARDOWN);
			}
		}
		return;
	}
kick:
	ev_kick(it, EV_D0 | EV_D1);
	return;
watch_abort:
	if (ev_decide(it, K_W_ABORTED, C_PEER, h->idx, 0, false, false)) {
		if (hazard & 4)
			ev_kick(it, EV_TEARDOWN);
		else {
			ev_queue_record(it);
			ev_kick(it, EV_TEARDOWN);
		}
	}
}

static void ev_sock_error(struct ovd_half *h, struct sock *sk)
{
	struct ovd_item *it = h->it;
	int err = READ_ONCE(sk->sk_err);

	if (!err)
		return;
	if (it->watch) {
		if (ev_decide(it, K_W_ABORTED, C_SOCKET, h->idx, err, false, false)) {
			if (!(hazard & 4))
				ev_queue_record(it);
			ev_kick(it, EV_TEARDOWN);
		}
	} else if (ev_decide(it, K_ABORTED, C_SOCKET, h->idx, err, false, false)) {
		ev_kick(it, EV_TEARDOWN);
	}
}

/* ---- socket callbacks ------------------------------------------------------------------ */
static void ev_data_ready(struct sock *sk)
{
	struct ovd_half *h;

	rcu_read_lock();
	h = rcu_dereference_sk_user_data(sk);
	if (h)
		ev_kick(h->it, h->idx ? EV_D1 : EV_D0);
	rcu_read_unlock();
}

static void ev_write_space(struct sock *sk)
{
	struct ovd_half *h;

	rcu_read_lock();
	h = rcu_dereference_sk_user_data(sk);
	if (h) {
		h->o_write_space(sk);
		if (h->vsock || sk_stream_is_writeable(sk))
			ev_kick(h->it, h->idx ? EV_D0 : EV_D1);
	}
	rcu_read_unlock();
}

static void ev_state_change(struct sock *sk)
{
	struct ovd_half *h;

	rcu_read_lock();
	h = rcu_dereference_sk_user_data(sk);
	if (h) {
		bool orig_first = !h->it->watch || (hazard & 8) || !h->vsock;

		if (orig_first)
			h->o_state_change(sk);
		if (h->vsock)
			ev_classify_vsock(h, sk);
		else
			ev_kick(h->it, EV_D0 | EV_D1);
		if (!orig_first)
			h->o_state_change(sk);
	}
	rcu_read_unlock();
}

static void ev_error_report(struct sock *sk)
{
	struct ovd_half *h;

	rcu_read_lock();
	h = rcu_dereference_sk_user_data(sk);
	if (h) {
		bool orig_first = !h->it->watch || (hazard & 8);

		if (orig_first)
			h->o_error_report(sk);
		/* A TCP reset (tcp_reset sets sk_err: ECONNRESET, or EPIPE in CLOSE_WAIT) is an abort even when the half's
		 * source direction already ended and nothing is left to recvmsg or sendmsg: sk_err is read here, not only
		 * discovered by an I/O call (hazard bit 4 (16) restores the I/O-only discovery). */
		if (h->vsock || !(hazard & 16))
			ev_sock_error(h, sk);
		if (!h->vsock)
			ev_kick(h->it, EV_D0 | EV_D1);
		if (!orig_first)
			h->o_error_report(sk);
	}
	rcu_read_unlock();
}

static int ev_hook(struct ovd_half *h)
{
	struct sock *sk = h->sk;
	int ret = 0;

	write_lock_bh(&sk->sk_callback_lock);
	if (sk->sk_user_data) {
		ret = -EBUSY;
		goto out;
	}
	h->o_data_ready = sk->sk_data_ready;
	h->o_write_space = sk->sk_write_space;
	h->o_state_change = sk->sk_state_change;
	h->o_error_report = sk->sk_error_report;
	rcu_assign_sk_user_data(sk, h);
	if (!h->it->watch) {
		WRITE_ONCE(sk->sk_data_ready, ev_data_ready);
		WRITE_ONCE(sk->sk_write_space, ev_write_space);
	}
	WRITE_ONCE(sk->sk_state_change, ev_state_change);
	WRITE_ONCE(sk->sk_error_report, ev_error_report);
	h->hooked = true;
out:
	write_unlock_bh(&sk->sk_callback_lock);
	return ret;
}

static void ev_unhook(struct ovd_half *h)
{
	struct sock *sk = h->sk;

	if (!h->hooked)
		return;
	write_lock_bh(&sk->sk_callback_lock);
	if (!h->it->watch) {
		WRITE_ONCE(sk->sk_data_ready, h->o_data_ready);
		WRITE_ONCE(sk->sk_write_space, h->o_write_space);
	}
	WRITE_ONCE(sk->sk_state_change, h->o_state_change);
	WRITE_ONCE(sk->sk_error_report, h->o_error_report);
	rcu_assign_sk_user_data(sk, NULL);
	h->hooked = false;
	write_unlock_bh(&sk->sk_callback_lock);
}

/* ---- forwarding ------------------------------------------------------------------------ */
enum { EV_BLOCKED = 0, EV_MORE = 1, EV_EOF = 2 };

static int ev_consume(struct socket *s, size_t n)
{
	struct msghdr m = { .msg_flags = MSG_DONTWAIT };
	int r;

	iov_iter_discard(&m.msg_iter, ITER_DEST, n);
	r = sock_recvmsg(s, &m, MSG_DONTWAIT);
	if (r < 0)
		return r;
	if ((size_t)r != n)
		return -EIO;
	return 0;
}

static int ev_send(struct socket *t, void *buf, int n)
{
	struct msghdr m = { .msg_flags = MSG_DONTWAIT | MSG_NOSIGNAL };
	struct kvec v = { .iov_base = buf, .iov_len = n };

	return kernel_sendmsg(t, &m, &v, 1, n);
}

static bool ev_decided(struct ovd_item *it)
{
	bool d;

	spin_lock_bh(&it->lock);
	d = it->decided;
	spin_unlock_bh(&it->lock);
	return d;
}

/* Returns EV_BLOCKED / EV_MORE / EV_EOF; a hard error decides an abort and returns EV_BLOCKED. */
static int ev_fwd(struct ovd_item *it, int d, void *buf, int *budget)
{
	struct ovd_half *s = &it->h[d], *t = &it->h[!d];

	if (it->src_end[d] != SE_NONE)
		return EV_BLOCKED;

	while (*budget > 0) {
		struct msghdr rm = { .msg_flags = MSG_DONTWAIT };
		struct kvec rv = { .iov_base = buf, .iov_len = OVD_CHUNK };
		int n, sent;

		n = kernel_recvmsg(s->sock, &rm, &rv, 1, OVD_CHUNK, MSG_DONTWAIT | MSG_PEEK);
		if (n == -EAGAIN)
			return EV_BLOCKED;
		if (n < 0) {
			ev_decide(it, K_ABORTED, C_SOCKET, s->idx, -n, false, false);
			return EV_BLOCKED;
		}
		if (n == 0) {
			bool clean = true;

			if (s->vsock && !(hazard & 1)) {
				if (hazard & 2) {
					clean = (vsock_sk(s->sk)->peer_shutdown & SHUTDOWN_MASK) != SHUTDOWN_MASK;
				} else if (!s->vclean) {
					if (!ev_decided(it)) {
						atomic64_inc(&it->ctx->unclassified_eof);
						ev_decide(it, K_ABORTED, C_PEER, s->idx, 0, false, false);
					}
					return EV_BLOCKED;
				}
			}
			if (!clean) {
				ev_decide(it, K_ABORTED, C_PEER, s->idx, 0, false, false);
				return EV_BLOCKED;
			}
			it->src_end[d] = SE_CLEAN;
			kernel_sock_shutdown(t->sock, SHUT_WR);
			it->tgt_shut[d] = true;
			return EV_EOF;
		}
		sent = ev_send(t->sock, buf, n);
		if (sent == -EAGAIN)
			return EV_BLOCKED;
		if (sent < 0) {
			ev_decide(it, K_ABORTED, C_SOCKET, t->idx, -sent, false, false);
			return EV_BLOCKED;
		}
		if (sent > 0) {
			int r = ev_consume(s->sock, sent);

			if (r) {
				ev_decide(it, K_ABORTED, C_SOCKET, s->idx, -r, false, false);
				return EV_BLOCKED;
			}
			it->moved[d] += sent;
			*budget -= sent;
		}
		if (sent < n)
			return EV_BLOCKED;
	}
	return EV_MORE;
}

static void ev_teardown(struct ovd_item *it)
{
	struct ovd_ctx *ctx = it->ctx;
	bool abort_act, completed;
	int i;

	spin_lock_bh(&it->lock);
	abort_act = it->d_kind == K_ABORTED && !it->d_noact;
	completed = it->d_kind == K_COMPLETED;
	spin_unlock_bh(&it->lock);

	if (abort_act) {
		for (i = 0; i < it->nh; i++) {
			struct ovd_half *h = &it->h[i];

			if (!h->sock)
				continue;
			if (h->vsock)
				kernel_sock_shutdown(h->sock, SHUT_RDWR);
			else
				tcp_abort(h->sk, ECONNABORTED);
		}
	} else if (it->watch && !it->d_noact && it->d_kind == K_W_ABORTED && it->d_cause == C_DETACHED) {
		kernel_sock_shutdown(it->h[0].sock, SHUT_RDWR);
	}
	for (i = 0; i < it->nh; i++)
		ev_unhook(&it->h[i]);
	for (i = 0; i < it->nh; i++) {
		if (it->h[i].sock) {
			sockfd_put(it->h[i].sock);
			it->h[i].sock = NULL;
		}
	}
	(void)completed;
	ev_count_end(it);
	if (!it->watch || (hazard & 4))
		ev_queue_record(it);
	if (it->d_norecord) {   /* context release or failed attach: leave the live list, no record */
		spin_lock_bh(&ctx->lock);
		if (!list_empty(&it->live_node)) {
			list_del_init(&it->live_node);
			if (it->watch)
				ctx->n_watches--;
			else
				ctx->n_pairs--;
		}
		spin_unlock_bh(&ctx->lock);
	}
	it->done = true;
	complete_all(&it->torn);
	ev_put(it);   /* the context's reference */
	atomic_dec(&ctx->nitems);   /* the last access to the context */
}

static void ev_work(struct work_struct *w)
{
	struct ovd_item *it = container_of(to_delayed_work(w), struct ovd_item, work);
	int budget[2] = { OVD_BUDGET, OVD_BUDGET };
	unsigned long again = 0, m;
	void *buf = NULL;
	int d;

	if (it->done)
		goto out;
	if (ev_decided(it)) {
		atomic_long_set(&it->pending, 0);
		ev_teardown(it);
		goto out;
	}
	if (it->watch) {   /* a watch forwards nothing; its decision arrives from the callback */
		atomic_long_set(&it->pending, 0);
		goto out;
	}
	while ((m = atomic_long_xchg(&it->pending, 0)) != 0) {
		if (m & EV_TEARDOWN)
			break;
		if (!buf) {
			buf = kvmalloc(OVD_CHUNK, GFP_KERNEL);
			if (!buf) {
				ev_decide(it, K_ABORTED, C_SOCKET, 0, ENOMEM, false, false);
				break;
			}
		}
		for (d = 0; d < 2; d++) {
			int r;

			if (!(m & (d ? EV_D1 : EV_D0)))
				continue;
			if (budget[d] <= 0) {
				again |= d ? EV_D1 : EV_D0;
				continue;
			}
			r = ev_fwd(it, d, buf, &budget[d]);
			if (r == EV_MORE)
				again |= d ? EV_D1 : EV_D0;
			if (ev_decided(it))
				break;
		}
		if (ev_decided(it) || again)
			break;
	}
	if (!ev_decided(it) && it->src_end[0] == SE_CLEAN && it->src_end[1] == SE_CLEAN && it->tgt_shut[0] && it->tgt_shut[1])
		ev_decide(it, K_COMPLETED, C_NONE, 0, 0, false, false);
	if (ev_decided(it)) {
		ev_teardown(it);
		goto out;
	}
	if (again)
		ev_kick(it, again);
out:
	kvfree(buf);
	ev_put(it);
}

/* ---- context and operations ------------------------------------------------------------- */
static int ev_init_half(struct ovd_half *h, struct ovd_item *it, int idx, int fd, u8 *bad)
{
	struct socket *s;
	int err;

	*bad = idx;
	s = sockfd_lookup(fd, &err);
	if (!s)
		return err;
	h->sock = s;
	h->sk = s->sk;
	h->it = it;
	h->idx = idx;
	if (s->sk->sk_family == AF_VSOCK && s->type == SOCK_STREAM) {
		h->vsock = true;
	} else if (s->sk->sk_family == AF_INET && s->sk->sk_protocol == IPPROTO_TCP) {
		h->vsock = false;
	} else {
		return -ESOCKTNOSUPPORT;
	}
	return 0;
}

static void ev_half_release(struct ovd_half *h)
{
	if (h->sock) {
		sockfd_put(h->sock);
		h->sock = NULL;
	}
}

static struct ovd_item *ev_new_item(struct ovd_ctx *ctx, bool watch)
{
	struct ovd_item *it = kzalloc(sizeof(*it), GFP_KERNEL);

	if (!it)
		return NULL;
	refcount_set(&it->ref, 1);   /* the context's reference */
	it->ctx = ctx;
	it->watch = watch;
	it->nh = watch ? 1 : 2;
	spin_lock_init(&it->lock);
	INIT_DELAYED_WORK(&it->work, ev_work);
	INIT_LIST_HEAD(&it->live_node);
	INIT_LIST_HEAD(&it->q_node);
	init_completion(&it->torn);
	return it;
}

static void ev_register(struct ovd_ctx *ctx, struct ovd_item *it)
{
	spin_lock_bh(&ctx->lock);
	it->id = ++ctx->next_id;
	atomic_inc(&ctx->nitems);
	list_add_tail(&it->live_node, &ctx->live);
	if (it->watch)
		ctx->n_watches++;
	else
		ctx->n_pairs++;
	spin_unlock_bh(&ctx->lock);
}

/* After a failed attach: no record, no socket action, wait until the work item tore the item down. */
static void ev_fail_item(struct ovd_item *it)
{
	ev_decide(it, K_ABORTED, C_DETACHED, 0, 0, true, true);
	ev_kick(it, EV_TEARDOWN);
	wait_for_completion(&it->torn);
}

static long ev_attach(struct ovd_ctx *ctx, struct ovd_fwd_attach __user *u)
{
	struct ovd_fwd_attach a;
	struct ovd_item *it;
	u8 bad = 0;
	long err;
	int i;

	if (copy_from_user(&a, u, sizeof(a)))
		return -EFAULT;
	if (a.flags || a.mode > 2)
		return -EINVAL;
	if (a.mode != 0)
		return -EOPNOTSUPP;   /* datagram mode is outside this spike */
	it = ev_new_item(ctx, false);
	if (!it)
		return -ENOMEM;
	err = ev_init_half(&it->h[0], it, 0, a.fd_a, &bad);
	if (err)
		goto fail_nohook;
	err = ev_init_half(&it->h[1], it, 1, a.fd_b, &bad);
	if (err)
		goto fail_nohook;
	if (it->h[0].sk == it->h[1].sk) {
		err = -EINVAL;
		goto fail_nohook;
	}
	if (it->h[0].vsock == it->h[1].vsock) {   /* exactly one vsock STREAM half */
		err = -ESOCKTNOSUPPORT;
		bad = it->h[0].vsock ? 1 : 0;
		goto fail_nohook;
	}
	for (i = 0; i < 2; i++) {
		struct sock *sk = it->h[i].sk;

		if (!it->h[i].vsock && inet_csk(sk)->icsk_ulp_ops) {
			err = -EPROTONOSUPPORT;
			bad = i;
			goto fail_nohook;
		}
		if (sk->sk_state != TCP_ESTABLISHED) {
			err = -ENOTCONN;
			bad = i;
			goto fail_nohook;
		}
	}
	ev_register(ctx, it);
	for (i = 0; i < 2; i++) {
		struct ovd_half *h = &it->h[i];

		if (h->vsock)
			lock_sock(h->sk);
		err = ev_hook(h);
		if (!err && h->vsock)
			ev_classify_vsock(h, h->sk);   /* an end that arrived before attach */
		if (h->vsock)
			release_sock(h->sk);
		if (err) {
			bad = i;
			a.bad_side = bad;
			ev_fail_item(it);
			if (put_user(bad, &u->bad_side))
				return -EFAULT;
			return err;
		}
	}
	a.id = it->id;
	atomic64_inc(&ctx->attached);
	if (copy_to_user(u, &a, sizeof(a))) {
		ev_decide(it, K_ABORTED, C_DETACHED, 0, 0, true, false);
		ev_kick(it, EV_TEARDOWN);
		return -EFAULT;
	}
	ev_kick(it, EV_D0 | EV_D1);   /* data or an end may already be queued on either half */
	return 0;
fail_nohook:
	for (i = 0; i < 2; i++)
		ev_half_release(&it->h[i]);
	kfree(it);
	if (put_user(bad, &u->bad_side))
		return -EFAULT;
	return err;
}

static long ev_watch(struct ovd_ctx *ctx, struct ovd_fwd_watch __user *u)
{
	struct ovd_fwd_watch w;
	struct ovd_item *it;
	u8 bad;
	long err;

	if (copy_from_user(&w, u, sizeof(w)))
		return -EFAULT;
	it = ev_new_item(ctx, true);
	if (!it)
		return -ENOMEM;
	err = ev_init_half(&it->h[0], it, 0, w.fd, &bad);
	if (err)
		goto fail_nohook;
	if (!it->h[0].vsock) {
		err = -ESOCKTNOSUPPORT;
		goto fail_nohook;
	}
	if (it->h[0].sk->sk_state != TCP_ESTABLISHED) {
		err = -ENOTCONN;
		goto fail_nohook;
	}
	ev_register(ctx, it);
	lock_sock(it->h[0].sk);
	err = ev_hook(&it->h[0]);
	if (!err)
		ev_classify_vsock(&it->h[0], it->h[0].sk);
	release_sock(it->h[0].sk);
	if (err) {
		ev_fail_item(it);
		return err;
	}
	w.id = it->id;
	atomic64_inc(&ctx->attached);
	if (copy_to_user(u, &w, sizeof(w))) {
		ev_decide(it, K_ABORTED, C_DETACHED, 0, 0, true, false);
		ev_kick(it, EV_TEARDOWN);
		return -EFAULT;
	}
	return 0;
fail_nohook:
	ev_half_release(&it->h[0]);
	kfree(it);
	return err;
}

static struct ovd_item *ev_find_live(struct ovd_ctx *ctx, u64 id)
{
	struct ovd_item *it, *found = NULL;

	spin_lock_bh(&ctx->lock);
	list_for_each_entry(it, &ctx->live, live_node) {
		if (it->id == id && refcount_inc_not_zero(&it->ref)) {
			found = it;
			break;
		}
	}
	spin_unlock_bh(&ctx->lock);
	return found;
}

static long ev_abort(struct ovd_ctx *ctx, u64 id)
{
	struct ovd_item *it = ev_find_live(ctx, id);

	if (!it)
		return -ENOENT;
	if (it->watch) {
		if (ev_decide(it, K_W_ABORTED, C_DETACHED, 0, 0, false, false)) {
			if (!(hazard & 4))
				ev_queue_record(it);
			ev_kick(it, EV_TEARDOWN);
		}
	} else if (ev_decide(it, K_ABORTED, C_DETACHED, 0, 0, false, false)) {
		ev_kick(it, EV_TEARDOWN);
	}
	ev_put(it);
	return 0;
}

static long ev_abort_all(struct ovd_ctx *ctx, u32 __user *u)
{
	struct ovd_item *it;
	u64 *ids;
	u32 n = 0, i = 0, cnt = 0;

	spin_lock_bh(&ctx->lock);
	list_for_each_entry(it, &ctx->live, live_node)
		n++;
	spin_unlock_bh(&ctx->lock);
	ids = kvmalloc_array(n + 64, sizeof(u64), GFP_KERNEL);
	if (!ids)
		return -ENOMEM;
	spin_lock_bh(&ctx->lock);
	list_for_each_entry(it, &ctx->live, live_node) {
		if (i < n + 64)
			ids[i++] = it->id;
	}
	spin_unlock_bh(&ctx->lock);
	for (n = 0; n < i; n++)
		if (ev_abort(ctx, ids[n]) == 0)
			cnt++;
	kvfree(ids);
	return put_user(cnt, u);
}

static long ev_ioctl(struct file *f, unsigned int cmd, unsigned long arg)
{
	struct ovd_ctx *ctx = f->private_data;
	void __user *u = (void __user *)arg;

	switch (cmd) {
	case OVD_FWD_VERSION: {
		struct ovd_fwd_version v = { .abi_version = OVD_FWD_ABI, .flags = OVD_FWD_FLAG_PERMANENT };

		return copy_to_user(u, &v, sizeof(v)) ? -EFAULT : 0;
	}
	case OVD_FWD_ATTACH:
		return ev_attach(ctx, u);
	case OVD_FWD_WATCH:
		return ev_watch(ctx, u);
	case OVD_FWD_ABORT: {
		u64 id;

		if (copy_from_user(&id, u, sizeof(id)))
			return -EFAULT;
		return ev_abort(ctx, id);
	}
	case OVD_FWD_ABORT_ALL:
		return ev_abort_all(ctx, u);
	case OVD_FWD_INVENTORY: {
		struct ovd_fwd_inventory v = {};

		spin_lock_bh(&ctx->lock);
		v.live_pairs = ctx->n_pairs;
		v.live_watches = ctx->n_watches;
		v.queued_ends = ctx->n_ends;
		spin_unlock_bh(&ctx->lock);
		v.attached = atomic64_read(&ctx->attached);
		v.completed = atomic64_read(&ctx->completed);
		v.aborted = atomic64_read(&ctx->aborted);
		v.unclassified_eof = atomic64_read(&ctx->unclassified_eof);
		v.ab_peer = atomic64_read(&ctx->ab_peer);
		v.ab_socket = atomic64_read(&ctx->ab_socket);
		v.ab_frame = atomic64_read(&ctx->ab_frame);
		v.ab_detached = atomic64_read(&ctx->ab_detached);
		return copy_to_user(u, &v, sizeof(v)) ? -EFAULT : 0;
	}
	case OVD_FWD_PAIR_STATE: {
		struct ovd_fwd_pair_state ps;
		struct ovd_item *it;
		int d;

		if (copy_from_user(&ps, u, sizeof(ps)))
			return -EFAULT;
		it = ev_find_live(ctx, ps.id);
		if (!it)
			return -ENOENT;
		for (d = 0; d < 2; d++) {
			ps.dir[d].source_end = it->src_end[d];
			ps.dir[d].target_shut = it->tgt_shut[d];
			ps.dir[d].moved = it->moved[d];
		}
		ev_put(it);
		return copy_to_user(u, &ps, sizeof(ps)) ? -EFAULT : 0;
	}
	}
	return -ENOTTY;
}

static ssize_t ev_read(struct file *f, char __user *buf, size_t len, loff_t *off)
{
	struct ovd_ctx *ctx = f->private_data;
	size_t done = 0;

	if (len < sizeof(struct ovd_fwd_end))
		return -EINVAL;
	for (;;) {
		struct ovd_item *it = NULL;
		struct ovd_fwd_end rec;

		spin_lock_bh(&ctx->lock);
		if (!list_empty(&ctx->ends)) {
			it = list_first_entry(&ctx->ends, struct ovd_item, q_node);
			list_del_init(&it->q_node);
			ctx->n_ends--;
			rec = it->rec;
		}
		spin_unlock_bh(&ctx->lock);
		if (!it) {
			if (done)
				return done;
			if (f->f_flags & O_NONBLOCK)
				return -EAGAIN;
			if (wait_event_interruptible(ctx->wq, READ_ONCE(ctx->n_ends) > 0))
				return -ERESTARTSYS;
			continue;
		}
		ev_put(it);
		if (copy_to_user(buf + done, &rec, sizeof(rec)))
			return done ? done : -EFAULT;
		done += sizeof(rec);
		if (len - done < sizeof(rec))
			return done;
	}
}

static __poll_t ev_poll(struct file *f, poll_table *wait)
{
	struct ovd_ctx *ctx = f->private_data;

	poll_wait(f, &ctx->wq, wait);
	return READ_ONCE(ctx->n_ends) ? EPOLLIN | EPOLLRDNORM : 0;
}

static int ev_open(struct inode *i, struct file *f)
{
	struct ovd_ctx *ctx = kzalloc(sizeof(*ctx), GFP_KERNEL);

	if (!ctx)
		return -ENOMEM;
	spin_lock_init(&ctx->lock);
	INIT_LIST_HEAD(&ctx->live);
	INIT_LIST_HEAD(&ctx->ends);
	init_waitqueue_head(&ctx->wq);
	f->private_data = ctx;
	return 0;
}

/* The release aborts every live pair and watch before it returns and queues no record. */
static int ev_release(struct inode *i, struct file *f)
{
	struct ovd_ctx *ctx = f->private_data;

	for (;;) {
		struct ovd_item *it = NULL, *c;

		spin_lock_bh(&ctx->lock);
		list_for_each_entry(c, &ctx->live, live_node) {
			if (refcount_inc_not_zero(&c->ref)) {
				it = c;
				break;
			}
		}
		spin_unlock_bh(&ctx->lock);
		if (!it)
			break;
		ev_decide(it, K_ABORTED, C_DETACHED, 0, 0, true, false);   /* no record */
		ev_kick(it, EV_TEARDOWN);
		wait_for_completion(&it->torn);
		ev_put(it);
	}
	/* wait until every item finished its teardown (its last access to the context) */
	while (atomic_read(&ctx->nitems))
		msleep(1);
	/* records another path queued before the release are dropped with the context */
	spin_lock_bh(&ctx->lock);
	ctx->released = true;
	while (!list_empty(&ctx->ends)) {
		struct ovd_item *q = list_first_entry(&ctx->ends, struct ovd_item, q_node);

		list_del_init(&q->q_node);
		ev_put(q);
	}
	spin_unlock_bh(&ctx->lock);
	kfree(ctx);
	return 0;
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

static int __init ev_init(void)
{
	int rc;

	ev_wq = alloc_workqueue("ovd_fwd", WQ_PERCPU, 0);
	if (!ev_wq)
		return -ENOMEM;
	rc = misc_register(&ev_misc);
	if (rc) {
		destroy_workqueue(ev_wq);
		return rc;
	}
	pr_info("ovd_fwd: loaded (permanent, abi %d, hazard=%u work_delay_us=%u)\n", OVD_FWD_ABI, hazard, work_delay_us);
	return 0;
}

module_init(ev_init);
/* No module_exit: delete_module returns -EBUSY (kernel/module/main.c:818-826 @ v7.0). */
MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("ev-end spike: proposed EVMOD contract, stream mode and watch");
