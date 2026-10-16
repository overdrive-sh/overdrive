// SPDX-License-Identifier: GPL-2.0
/*
 * evmod: event-driven in-kernel forwarding between two connected stream sockets
 * (AF_VSOCK <-> TCP, TCP <-> TCP). Spike code, not production.
 *
 * No per-peer kernel thread. Each socket's sk_data_ready / sk_write_space /
 * sk_state_change / sk_error_report is overridden (saved and restored). The
 * overrides only set a pending mask and queue ONE work item per pair on a shared
 * workqueue (one worker pool per CPU). The work moves bytes with
 *   kernel_recvmsg(MSG_PEEK|MSG_DONTWAIT) -> kernel_sendmsg(MSG_DONTWAIT) ->
 *   sock_recvmsg(discard iterator of exactly the bytes the target accepted)
 * so a byte leaves the source's receive queue (and the vsock credit is returned,
 * virtio_transport_common.c:583-656) only after the target socket accepted it.
 * When the target is full the work stops and resumes on the target's
 * sk_write_space. The module holds no payload between work runs.
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
#include <linux/vm_sockets.h>
#include <linux/rcupdate.h>
#include <net/sock.h>
#include <net/tcp.h>
#include "evmod_uapi.h"

#define EV_POOL      256
static unsigned int buf_kb = 64;
module_param(buf_kb, uint, 0444);
MODULE_PARM_DESC(buf_kb, "scratch/peek size in KiB (default 64, max 1024)");
static unsigned int EV_BUF = 65536U;                 /* peek size = scratch buffer size (fixed pool) */
#define EV_BUDGET    (4U * EV_BUF)   /* bytes per direction per work run, then requeue (fairness) */

static int fault;
module_param(fault, int, 0644);
MODULE_PARM_DESC(fault, "negative-control fault injection: 0 none, 1 drop 1 byte, 2 duplicate 8 bytes, 3 swap two 8-byte words");
static unsigned long fault_after = 1048576;
module_param(fault_after, ulong, 0644);
static int unbound;
module_param(unbound, int, 0444);
MODULE_PARM_DESC(unbound, "1 = WQ_UNBOUND workqueue, default per-CPU");

#define EV_D0        1UL   /* direction 0: h[0] -> h[1] needs a run */
#define EV_D1        2UL   /* direction 1: h[1] -> h[0] needs a run */
#define EV_TEARDOWN  4UL

struct ev_pair;
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
	bool src_done;     /* EOF seen on this half and propagated as shutdown(WR) on the other half */
};
struct ev_pair {
	refcount_t ref;
	struct rcu_head rcu;
	struct work_struct work;
	atomic_long_t pending;
	struct ev_half h[2];
	u32 id;
	bool dead;
	bool fault_done;
	u64 moved[2];
	struct completion torn;
};

static struct workqueue_struct *ev_wq;
static DEFINE_XARRAY_ALLOC(ev_pairs);
static void *ev_pool[EV_POOL];
static int ev_pool_n;
static DEFINE_SPINLOCK(ev_pool_lock);

static atomic64_t st_attached, st_detached, st_completed, st_aborted, st_runs, st_kicks, st_queued;
static atomic64_t st_bytes_v2t, st_bytes_t2v, st_bytes_t2t, st_stall_dst, st_stall_src, st_requeue;
static atomic64_t st_pool_miss, st_short_consume, st_fault_hits, st_err, st_eof;
static atomic_t st_held_max;

static void ev_put(struct ev_pair *p)
{
	if (refcount_dec_and_test(&p->ref))
		kfree_rcu(p, rcu);
}

/* Callable from any context (softirq, process). */
static void ev_kick(struct ev_pair *p, unsigned long mask)
{
	atomic64_inc(&st_kicks);
	atomic_long_or(mask, &p->pending);
	if (!refcount_inc_not_zero(&p->ref))
		return;
	if (queue_work(ev_wq, &p->work))
		atomic64_inc(&st_queued);
	else
		ev_put(p);
}

/* ---- socket callbacks ---------------------------------------------------------------- */
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
		/* TCP: the original clears SOCK_NOSPACE only if the socket is writeable. */
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
		h->orig_error_report(sk);
		ev_kick(h->pair, EV_D0 | EV_D1);
	}
	rcu_read_unlock();
}

static int ev_hook(struct ev_half *h)
{
	struct sock *sk = h->sk;
	int ret = 0;

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
	WRITE_ONCE(sk->sk_data_ready, ev_data_ready);
	WRITE_ONCE(sk->sk_write_space, ev_write_space);
	WRITE_ONCE(sk->sk_state_change, ev_state_change);
	WRITE_ONCE(sk->sk_error_report, ev_error_report);
	h->hooked = true;
out:
	write_unlock_bh(&sk->sk_callback_lock);
	return ret;
}

static void ev_unhook(struct ev_half *h)
{
	struct sock *sk = h->sk;

	if (!h->hooked)
		return;
	write_lock_bh(&sk->sk_callback_lock);
	WRITE_ONCE(sk->sk_data_ready, h->orig_data_ready);
	WRITE_ONCE(sk->sk_write_space, h->orig_write_space);
	WRITE_ONCE(sk->sk_state_change, h->orig_state_change);
	WRITE_ONCE(sk->sk_error_report, h->orig_error_report);
	rcu_assign_sk_user_data(sk, NULL);
	h->hooked = false;
	write_unlock_bh(&sk->sk_callback_lock);
}

/* ---- scratch pool (fixed, so module memory does not grow with traffic) ---------------- */
static void *ev_buf_get(void)
{
	void *b = NULL;

	spin_lock_bh(&ev_pool_lock);
	if (ev_pool_n)
		b = ev_pool[--ev_pool_n];
	spin_unlock_bh(&ev_pool_lock);
	if (!b) {
		atomic64_inc(&st_pool_miss);
		b = kvmalloc(EV_BUF, GFP_KERNEL);
	}
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

/* ---- forwarding ---------------------------------------------------------------------- */
enum { EV_BLOCKED = 0, EV_MORE = 1, EV_EOF = 2 };

static int ev_consume(struct socket *s, size_t n)
{
	struct msghdr m = { .msg_flags = MSG_DONTWAIT };
	int r;

	iov_iter_discard(&m.msg_iter, ITER_DEST, n);
	r = sock_recvmsg(s, &m, MSG_DONTWAIT);
	if (r < 0)
		return r;
	if ((size_t)r != n) {
		atomic64_inc(&st_short_consume);
		return -EIO;
	}
	return 0;
}

static int ev_send(struct socket *t, void *buf, int n)
{
	struct msghdr m = { .msg_flags = MSG_DONTWAIT | MSG_NOSIGNAL };
	struct kvec v = { .iov_base = buf, .iov_len = n };

	return kernel_sendmsg(t, &m, &v, 1, n);
}

/* Returns EV_BLOCKED / EV_MORE / EV_EOF, or a negative errno (pair must be aborted). */
static int ev_fwd(struct ev_pair *p, int d, void *buf, int *budget)
{
	struct ev_half *s = &p->h[d], *t = &p->h[!d];
	atomic64_t *ctr = s->tcp ? (t->tcp ? &st_bytes_t2t : &st_bytes_t2v) : &st_bytes_v2t;

	if (s->src_done)
		return EV_BLOCKED;

	while (*budget > 0) {
		struct msghdr rm = { .msg_flags = MSG_DONTWAIT };
		struct kvec rv = { .iov_base = buf, .iov_len = EV_BUF };
		int n, sent, skip = 0, pre = 0;
		void *sbuf = buf;

		n = kernel_recvmsg(s->sock, &rm, &rv, 1, EV_BUF, MSG_DONTWAIT | MSG_PEEK);
		if (n == -EAGAIN) {
			atomic64_inc(&st_stall_src);
			return EV_BLOCKED;
		}
		if (n < 0)
			return n;
		if (n == 0) {
			s->src_done = true;
			atomic64_inc(&st_eof);
			kernel_sock_shutdown(t->sock, SHUT_WR);
			return EV_EOF;
		}
		if (atomic_read(&st_held_max) < n)
			atomic_set(&st_held_max, n);

		/* negative-control faults, once per pair, only after fault_after bytes */
		if (fault && !p->fault_done && p->moved[d] >= fault_after) {
			if (fault == 1 && n >= 2) {          /* lose one byte */
				skip = 1;
				sbuf = buf + 1;
				n -= 1;
				p->fault_done = true;
			} else if (fault == 2 && n >= 8) {   /* duplicate the next 8 bytes */
				if (ev_send(t->sock, buf, 8) == 8) {
					pre = 8;
					p->fault_done = true;
				}
			} else if (fault == 3 && n >= 16) {  /* reorder two adjacent 8-byte words */
				u64 a, b;

				memcpy(&a, buf, 8);
				memcpy(&b, buf + 8, 8);
				memcpy(buf, &b, 8);
				memcpy(buf + 8, &a, 8);
				p->fault_done = true;
			}
			if (p->fault_done)
				atomic64_inc(&st_fault_hits);
		}
		(void)pre;

		sent = ev_send(t->sock, sbuf, n);
		if (sent == -EAGAIN) {
			atomic64_inc(&st_stall_dst);
			return EV_BLOCKED;
		}
		if (sent < 0)
			return sent;
		if (sent > 0) {
			int r = ev_consume(s->sock, sent + skip);

			if (r)
				return r;
			p->moved[d] += sent;
			atomic64_add(sent, ctr);
			*budget -= sent;
		}
		if (sent < n) {
			atomic64_inc(&st_stall_dst);
			return EV_BLOCKED;
		}
	}
	return EV_MORE;
}

static void ev_teardown(struct ev_pair *p, bool aborted)
{
	int i;

	p->dead = true;
	for (i = 0; i < 2; i++)
		ev_unhook(&p->h[i]);
	xa_erase(&ev_pairs, p->id);
	for (i = 0; i < 2; i++) {
		if (p->h[i].sock) {
			sockfd_put(p->h[i].sock);
			p->h[i].sock = NULL;
		}
	}
	if (aborted)
		atomic64_inc(&st_aborted);
	else
		atomic64_inc(&st_completed);
	complete_all(&p->torn);
	ev_put(p);   /* the xarray's reference */
}

static void ev_work(struct work_struct *w)
{
	struct ev_pair *p = container_of(w, struct ev_pair, work);
	int budget[2] = { EV_BUDGET, EV_BUDGET };
	unsigned long again = 0, m;
	void *buf = NULL;
	int err = 0, d;

	atomic64_inc(&st_runs);
	if (p->dead)
		goto out;

	while ((m = atomic_long_xchg(&p->pending, 0)) != 0) {
		if (m & EV_TEARDOWN) {
			ev_teardown(p, false);
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
		if (err)
			break;
		if (again)
			break;
	}
	if (err) {
		int i;

		atomic64_inc(&st_err);
		/* An error is an abort, not an orderly close: a TCP peer must see a reset (a FIN would leave a blocked sender
		 * blocked), a vsock peer sees the shutdown. tcp_abort() is EXPORT_SYMBOL_GPL (net/ipv4/tcp.c:5141). */
		for (i = 0; i < 2; i++) {
			if (p->h[i].tcp)
				tcp_abort(p->h[i].sk, ECONNABORTED);
			else
				kernel_sock_shutdown(p->h[i].sock, SHUT_RDWR);
		}
		ev_teardown(p, true);
		goto out;
	}
	if (p->h[0].src_done && p->h[1].src_done) {
		ev_teardown(p, false);
		goto out;
	}
	if (again) {
		atomic64_inc(&st_requeue);
		ev_kick(p, again);
	}
out:
	if (buf)
		ev_buf_put(buf);
	ev_put(p);
}

/* ---- control device ------------------------------------------------------------------ */
static int ev_init_half(struct ev_half *h, struct ev_pair *p, int idx, int fd)
{
	struct socket *s;
	int err;

	s = sockfd_lookup(fd, &err);
	if (!s)
		return err;
	h->sock = s;
	h->sk = s->sk;
	h->pair = p;
	h->idx = idx;
	if (s->sk->sk_family == AF_VSOCK && s->type == SOCK_STREAM) {
		h->tcp = false;
	} else if (s->sk->sk_family == AF_INET && s->sk->sk_protocol == IPPROTO_TCP) {
		h->tcp = true;
	} else {
		sockfd_put(s);
		h->sock = NULL;
		return -EOPNOTSUPP;
	}
	if (s->sk->sk_state != TCP_ESTABLISHED) {
		sockfd_put(s);
		h->sock = NULL;
		return -ENOTCONN;
	}
	return 0;
}

/* teardown through the work item so nothing races a callback that already fired */
static void ev_abort_async(struct ev_pair *p)
{
	atomic_long_or(EV_TEARDOWN, &p->pending);
	refcount_inc(&p->ref);
	if (!queue_work(ev_wq, &p->work))
		ev_put(p);
}

static long ev_attach(struct evmod_attach __user *u)
{
	struct evmod_attach a;
	struct ev_pair *p;
	int err, i;

	if (copy_from_user(&a, u, sizeof(a)))
		return -EFAULT;
	if (a.flags)
		return -EINVAL;
	p = kzalloc(sizeof(*p), GFP_KERNEL);
	if (!p)
		return -ENOMEM;
	refcount_set(&p->ref, 1);   /* the xarray's reference */
	INIT_WORK(&p->work, ev_work);
	init_completion(&p->torn);
	err = ev_init_half(&p->h[0], p, 0, a.fd_a);
	if (err)
		goto fail;
	err = ev_init_half(&p->h[1], p, 1, a.fd_b);
	if (err)
		goto fail;
	if (p->h[0].sk == p->h[1].sk) {
		err = -EINVAL;
		goto fail;
	}
	err = xa_alloc(&ev_pairs, &p->id, p, xa_limit_31b, GFP_KERNEL);
	if (err)
		goto fail;
	for (i = 0; i < 2; i++) {
		err = ev_hook(&p->h[i]);
		if (err) {
			ev_abort_async(p);
			return err;
		}
	}
	a.id = p->id;
	if (copy_to_user(u, &a, sizeof(a))) {
		ev_abort_async(p);
		return -EFAULT;
	}
	atomic64_inc(&st_attached);
	ev_kick(p, EV_D0 | EV_D1);   /* data may already be queued on either side */
	return 0;
fail:
	for (i = 0; i < 2; i++)
		if (p->h[i].sock)
			sockfd_put(p->h[i].sock);
	kfree(p);
	return err;
}

static long ev_detach(u32 id)
{
	struct ev_pair *p;
	long r = 0;

	xa_lock(&ev_pairs);
	p = xa_load(&ev_pairs, id);
	if (p && !refcount_inc_not_zero(&p->ref))
		p = NULL;
	xa_unlock(&ev_pairs);
	if (!p)
		return -ENOENT;
	ev_kick(p, EV_TEARDOWN);
	if (!wait_for_completion_timeout(&p->torn, 20 * HZ))
		r = -ETIMEDOUT;
	else
		atomic64_inc(&st_detached);
	ev_put(p);
	return r;
}

static long ev_ioctl(struct file *f, unsigned int cmd, unsigned long arg)
{
	switch (cmd) {
	case EVMOD_ATTACH:
		return ev_attach((struct evmod_attach __user *)arg);
	case EVMOD_DETACH: {
		u32 id;

		if (copy_from_user(&id, (u32 __user *)arg, sizeof(id)))
			return -EFAULT;
		return ev_detach(id);
	}
	}
	return -ENOTTY;
}

static const struct file_operations ev_fops = {
	.owner = THIS_MODULE,
	.unlocked_ioctl = ev_ioctl,
	.compat_ioctl = ev_ioctl,
};
static struct miscdevice ev_misc = { .minor = MISC_DYNAMIC_MINOR, .name = "evmod", .fops = &ev_fops };

static int ev_stats(struct seq_file *m, void *v)
{
	unsigned long live = 0, idx;
	void *e;

	xa_for_each(&ev_pairs, idx, e)
		live++;
	seq_printf(m,
		   "live=%lu attached=%lld detached=%lld completed=%lld aborted=%lld err=%lld runs=%lld kicks=%lld queued=%lld requeue=%lld "
		   "bytes_v2t=%lld bytes_t2v=%lld bytes_t2t=%lld stall_dst=%lld stall_src=%lld pool_miss=%lld short_consume=%lld "
		   "fault_hits=%lld eof_fwd=%lld held_max=%d pair_size=%zu buf_bytes=%u\n",
		   live, atomic64_read(&st_attached), atomic64_read(&st_detached), atomic64_read(&st_completed), atomic64_read(&st_aborted),
		   atomic64_read(&st_err), atomic64_read(&st_runs), atomic64_read(&st_kicks), atomic64_read(&st_queued), atomic64_read(&st_requeue),
		   atomic64_read(&st_bytes_v2t), atomic64_read(&st_bytes_t2v), atomic64_read(&st_bytes_t2t), atomic64_read(&st_stall_dst),
		   atomic64_read(&st_stall_src), atomic64_read(&st_pool_miss), atomic64_read(&st_short_consume), atomic64_read(&st_fault_hits),
		   atomic64_read(&st_eof), atomic_read(&st_held_max), sizeof(struct ev_pair), EV_BUF);
	return 0;
}

static int __init ev_init(void)
{
	int i, rc;
	unsigned int flags = WQ_PERCPU;

	if (buf_kb < 4 || buf_kb > 1024)
		return -EINVAL;
	EV_BUF = buf_kb * 1024U;

	if (unbound)
		flags = WQ_UNBOUND;
	ev_wq = alloc_workqueue("evmod", flags, 0);
	if (!ev_wq)
		return -ENOMEM;
	for (i = 0; i < 8; i++) {   /* a few preallocated buffers; the rest are taken on demand and kept */
		void *b = kvmalloc(EV_BUF, GFP_KERNEL);

		if (!b) {
			rc = -ENOMEM;
			goto fail_wq;
		}
		ev_pool[ev_pool_n++] = b;
	}
	rc = misc_register(&ev_misc);
	if (rc)
		goto fail_wq;
	proc_create_single("evmod", 0444, NULL, ev_stats);
	pr_info("evmod: loaded (%s workqueue)\n", unbound ? "unbound" : "per-cpu");
	return 0;
fail_wq:
	while (ev_pool_n)
		kvfree(ev_pool[--ev_pool_n]);
	destroy_workqueue(ev_wq);
	return rc;
}

static void __exit ev_exit(void)
{
	unsigned long idx;
	struct ev_pair *p;

	remove_proc_entry("evmod", NULL);
	misc_deregister(&ev_misc);
	for (;;) {
		bool any = false;

		xa_for_each(&ev_pairs, idx, p) {
			any = true;
			ev_detach(idx);
		}
		if (!any)
			break;
	}
	synchronize_rcu();
	destroy_workqueue(ev_wq);
	rcu_barrier();
	while (ev_pool_n)
		kvfree(ev_pool[--ev_pool_n]);
	pr_info("evmod: unloaded attached=%lld completed=%lld detached=%lld aborted=%lld\n", atomic64_read(&st_attached),
		atomic64_read(&st_completed), atomic64_read(&st_detached), atomic64_read(&st_aborted));
}

module_init(ev_init);
module_exit(ev_exit);
MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("event-driven vsock/TCP forwarder (spike)");
