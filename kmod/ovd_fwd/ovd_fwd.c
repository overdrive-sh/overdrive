// SPDX-License-Identifier: GPL-2.0
#include <linux/atomic.h>
#include <linux/bits.h>
#include <linux/build_bug.h>
#include <linux/completion.h>
#include <linux/errno.h>
#include <linux/fs.h>
#include <linux/init.h>
#include <linux/inet.h>
#include <linux/miscdevice.h>
#include <linux/mempool.h>
#include <linux/module.h>
#include <linux/net_tstamp.h>
#include <linux/poll.h>
#include <linux/refcount.h>
#include <linux/slab.h>
#include <linux/uaccess.h>
#include <linux/uio.h>
#include <linux/vm_sockets.h>
#include <linux/wait.h>
#include <linux/workqueue.h>
#include <net/af_vsock.h>
#include <net/inet_connection_sock.h>
#include <net/sock.h>
#include <net/tcp.h>

#include <linux/ovd_fwd.h>
#include "ovd_fwd_frame.h"
#include "ovd_fwd_frame.c"

#ifndef OVD_FWD_MODULE_VERSION
#error "OVD_FWD_MODULE_VERSION must be supplied by cargo xtask kmod-build"
#endif

#define FWD_CHUNK 65536U
#define FWD_RUN_BUDGET (4U * FWD_CHUNK)
#define FWD_SCRATCH_POOL 8U

#define EV_DIR_A BIT(0)
#define EV_DIR_B BIT(1)
#define EV_FINISH BIT(2)

#define END_NONE 0U
#define END_COMPLETED 1U
#define END_ABORTED 2U
#define END_WATCH_HALF_CLOSE 3U
#define END_WATCH_ABORTED 4U

#define CAUSE_NONE 0U
#define CAUSE_PEER 1U
#define CAUSE_SOCKET 2U
#define CAUSE_FRAME 3U
#define CAUSE_DETACHED 4U
#define CAUSE_UNCLASSIFIED 5U

#define SOURCE_NONE 0U
#define SOURCE_CLEAN 1U
#define SOURCE_ABORT 2U

enum ovd_fwd_kind {
	OVD_KIND_OTHER,
	OVD_KIND_VSOCK,
	OVD_KIND_TCP,
	OVD_KIND_UDP,
};

struct ovd_fwd_ctx;
struct ovd_fwd_item;

struct ovd_fwd_half {
	struct socket *sock;
	struct sock *sk;
	struct ovd_fwd_item *item;
	struct ovd_fwd_ctx *ctx;
	wait_queue_entry_t write_wait;
	void (*orig_data_ready)(struct sock *sk);
	void (*orig_state_change)(struct sock *sk);
	void (*orig_error_report)(struct sock *sk);
	u8 side;
	u8 kind;
	u8 acted_peer;
	u8 source_end;
	bool hooked;
	bool wait_added;
	bool module_ref;
	bool keep_module;
};

struct ovd_fwd_direction {
	u8 source_end;
	bool target_shut;
	u64 moved;
	u32 dgram_sent;
	u32 dgram_need;
	bool dgram_started;
};

struct ovd_fwd_item {
	struct list_head live_node;
	struct list_head end_node;
	struct work_struct work;
	struct rcu_work free_work;
	refcount_t refs;
	atomic_t work_refs;
	atomic_long_t pending;
	struct ovd_fwd_ctx *ctx;
	struct ovd_fwd_half half[2];
	struct ovd_fwd_direction dir[2];
	struct ovd_fwd_end record;
	u64 id;
	u32 mode;
	u8 end_kind;
	u8 end_cause;
	u8 end_side;
	u16 end_errno;
	int err_side;
	bool watch;
	bool ready;
	bool dead;
	bool record_queued;
	bool releasing;
	bool cleanup_queued;
	bool free_work_queued;
};

struct ovd_fwd_ctx {
	spinlock_t lock; /* protects ids, end records and inventory counts */
	struct mutex ops_lock; /* serializes context management operations */
	struct mutex read_lock; /* serializes record reads */
	struct list_head live;
	struct list_head ends;
	wait_queue_head_t wait;
	atomic_t outstanding;
	atomic_t cleanup_pending;
	u64 next_id;
	u32 live_pairs;
	u32 live_watches;
	u32 queued_ends;
	bool releasing;
	atomic64_t attached;
	atomic64_t completed;
	atomic64_t aborted_peer;
	atomic64_t aborted_socket;
	atomic64_t aborted_frame;
	atomic64_t aborted_detached;
	atomic64_t unclassified_eof;
	atomic64_t dgram_oversize;
};

static struct workqueue_struct *ovd_fwd_workqueue;
static mempool_t *ovd_fwd_scratch_pool;

static void ovd_fwd_work(struct work_struct *work);
static void ovd_fwd_free_work(struct work_struct *work);
static void ovd_fwd_item_put(struct ovd_fwd_item *item);
static void ovd_fwd_kick(struct ovd_fwd_item *item, unsigned long bits);
static long ovd_fwd_abort_id(struct ovd_fwd_ctx *ctx, u64 id);
static long ovd_fwd_inventory(struct ovd_fwd_ctx *ctx,
			      struct ovd_fwd_inventory __user *user);
static long ovd_fwd_pair_state(struct ovd_fwd_ctx *ctx,
			       struct ovd_fwd_pair_state __user *user);
static bool ovd_fwd_record_watch_locked(struct ovd_fwd_item *item, u8 kind,
					u8 cause, u8 side, u16 error);

static_assert(sizeof(struct ovd_fwd_end) == 32);
static_assert(offsetof(struct ovd_fwd_end, id) == 0);
static_assert(offsetof(struct ovd_fwd_end, kind) == 8);
static_assert(offsetof(struct ovd_fwd_end, cause) == 9);
static_assert(offsetof(struct ovd_fwd_end, side) == 10);
static_assert(offsetof(struct ovd_fwd_end, errno) == 12);
static_assert(offsetof(struct ovd_fwd_end, moved_ab) == 16);
static_assert(offsetof(struct ovd_fwd_end, moved_ba) == 24);
static_assert(OVD_FWD_FRAME_MAX_PAYLOAD + OVD_FWD_FRAME_HDR_LEN ==
	      OVD_FWD_FRAME_MAX);

static void ovd_fwd_item_get(struct ovd_fwd_item *item)
{
	refcount_inc(&item->refs);
}

static void ovd_fwd_item_put(struct ovd_fwd_item *item)
{
	struct ovd_fwd_ctx *ctx;

	if (!refcount_dec_and_test(&item->refs))
		return;
	ctx = item->ctx;
	atomic_dec(&ctx->outstanding);
	wake_up_all(&ctx->wait);
	kfree(item);
}

static void ovd_fwd_kick(struct ovd_fwd_item *item, unsigned long bits)
{
	atomic_long_or(bits, &item->pending);
	if (!READ_ONCE(item->ready))
		return;
	ovd_fwd_item_get(item);
	atomic_inc(&item->work_refs);
	if (!queue_work(ovd_fwd_workqueue, &item->work)) {
		atomic_dec(&item->work_refs);
		ovd_fwd_item_put(item);
	}
}

static void ovd_fwd_schedule_free(struct ovd_fwd_item *item)
{
	struct ovd_fwd_ctx *ctx = item->ctx;

	if (xchg(&item->free_work_queued, true))
		return;
	ovd_fwd_item_get(item);
	if (!item->cleanup_queued) {
		item->cleanup_queued = true;
		atomic_inc(&ctx->cleanup_pending);
	}
	queue_rcu_work(ovd_fwd_workqueue, &item->free_work);
}

static void ovd_fwd_cleanup_begin_locked(struct ovd_fwd_item *item)
{
	if (!item->cleanup_queued) {
		item->cleanup_queued = true;
		atomic_inc(&item->ctx->cleanup_pending);
	}
}

static void ovd_fwd_record_fill(struct ovd_fwd_item *item)
{
	memset(&item->record, 0, sizeof(item->record));
	item->record.id = item->id;
	item->record.kind = item->end_kind;
	item->record.cause = item->end_cause;
	item->record.side = item->end_side;
	item->record.errno = item->end_errno;
	item->record.moved_ab = item->dir[0].moved;
	item->record.moved_ba = item->dir[1].moved;
}

static void ovd_fwd_counter_end(struct ovd_fwd_ctx *ctx, u8 kind, u8 cause)
{
	if (kind == END_COMPLETED) {
		atomic64_inc(&ctx->completed);
		return;
	}
	if (kind != END_ABORTED && kind != END_WATCH_ABORTED)
		return;
	switch (cause) {
	case CAUSE_PEER:
		atomic64_inc(&ctx->aborted_peer);
		break;
	case CAUSE_SOCKET:
		atomic64_inc(&ctx->aborted_socket);
		break;
	case CAUSE_FRAME:
		atomic64_inc(&ctx->aborted_frame);
		break;
	case CAUSE_DETACHED:
		atomic64_inc(&ctx->aborted_detached);
		break;
	case CAUSE_UNCLASSIFIED:
		atomic64_inc(&ctx->unclassified_eof);
		break;
	default:
		break;
	}
}

static bool ovd_fwd_watch_end_locked(struct ovd_fwd_half *half, u8 kind,
				     u8 cause)
{
	return ovd_fwd_record_watch_locked(half->item, kind, cause, half->side, 0);
}

static bool ovd_fwd_request_abort(struct ovd_fwd_item *item, u8 cause,
				  u8 side, u16 error)
{
	struct ovd_fwd_ctx *ctx = item->ctx;
	unsigned long flags;
	bool won = false;

	spin_lock_irqsave(&ctx->lock, flags);
	if (!item->dead && item->end_kind == END_NONE) {
		item->end_kind = END_ABORTED;
		item->end_cause = cause;
		item->end_side = side;
		item->end_errno = error;
		if (cause != CAUSE_DETACHED && side < 2) {
			item->half[side].source_end = SOURCE_ABORT;
			item->dir[side].source_end = SOURCE_ABORT;
		}
		if (ctx->releasing || item->releasing)
			item->releasing = true;
		won = true;
	}
	spin_unlock_irqrestore(&ctx->lock, flags);
	if (won)
		ovd_fwd_kick(item, EV_FINISH);
	return won;
}

static bool ovd_fwd_request_complete(struct ovd_fwd_item *item)
{
	struct ovd_fwd_ctx *ctx = item->ctx;
	unsigned long flags;
	bool won = false;

	spin_lock_irqsave(&ctx->lock, flags);
	if (!item->dead && item->end_kind == END_NONE) {
		item->end_kind = END_COMPLETED;
		item->end_cause = CAUSE_NONE;
		won = true;
	}
	spin_unlock_irqrestore(&ctx->lock, flags);
	return won;
}

static bool ovd_fwd_pair_completed(struct ovd_fwd_item *item)
{
	unsigned long flags;
	bool completed;

	spin_lock_irqsave(&item->ctx->lock, flags);
	completed = item->end_kind == END_COMPLETED;
	spin_unlock_irqrestore(&item->ctx->lock, flags);
	return completed;
}

static bool ovd_fwd_record_watch_locked(struct ovd_fwd_item *item, u8 kind,
					u8 cause, u8 side, u16 error)
{
	struct ovd_fwd_ctx *ctx = item->ctx;

	if (item->record_queued || item->dead)
		return false;
	item->end_kind = kind;
	item->end_cause = cause;
	item->end_side = side;
	item->end_errno = error;
	if (!item->ready)
		return false;
	if (item->releasing || ctx->releasing) {
		item->releasing = true;
		ovd_fwd_cleanup_begin_locked(item);
		if (!list_empty(&item->live_node)) {
			list_del_init(&item->live_node);
			ctx->live_watches--;
		}
		ovd_fwd_kick(item, EV_FINISH);
		return false;
	}
	item->record_queued = true;
	ovd_fwd_cleanup_begin_locked(item);
	ovd_fwd_record_fill(item);
	list_del_init(&item->live_node);
	ctx->live_watches--;
	list_add_tail(&item->end_node, &ctx->ends);
	ctx->queued_ends++;
	ovd_fwd_counter_end(ctx, kind, cause);
	ovd_fwd_kick(item, EV_FINISH);
	return true;
}

static bool ovd_fwd_record_watch(struct ovd_fwd_item *item, u8 kind, u8 cause,
				 u8 side, u16 error)
{
	struct ovd_fwd_ctx *ctx = item->ctx;
	unsigned long flags;
	bool queued;

	spin_lock_irqsave(&ctx->lock, flags);
	queued = ovd_fwd_record_watch_locked(item, kind, cause, side, error);
	spin_unlock_irqrestore(&ctx->lock, flags);
	wake_up_all(&ctx->wait);
	if (queued)
		wake_up_interruptible(&ctx->wait);
	return queued;
}

static int ovd_fwd_write_wake(wait_queue_entry_t *wait, unsigned int mode,
			      int sync, void *key)
{
	struct ovd_fwd_half *half = container_of(wait, struct ovd_fwd_half, write_wait);
	unsigned long events = key ? (unsigned long)key : ~0UL;
	unsigned long direction;

	(void)mode;
	(void)sync;
	if (!(events & (__force unsigned long)(EPOLLOUT | EPOLLWRNORM | EPOLLWRBAND)))
		return 0;
	direction = half->side == 0 ? EV_DIR_B : EV_DIR_A;
	ovd_fwd_kick(half->item, direction);
	return 1;
}

static void ovd_fwd_classify_vsock(struct ovd_fwd_half *half)
{
	struct ovd_fwd_item *item = half->item;
	struct ovd_fwd_ctx *ctx = item->ctx;
	struct sock *sk = half->sk;
	u8 peer = READ_ONCE(vsock_sk(sk)->peer_shutdown) & SHUTDOWN_MASK;
	u8 fresh;
	u8 watch_kind = 0;
	u8 watch_cause = 0;
	bool queue_direction = false;
	bool queue_cleanup = false;
	unsigned long flags;

	spin_lock_irqsave(&ctx->lock, flags);
	if (item->dead || item->record_queued ||
	    (item->watch && item->end_kind != END_NONE) ||
	    (!item->watch && item->end_kind == END_ABORTED)) {
		spin_unlock_irqrestore(&ctx->lock, flags);
		return;
	}
	fresh = peer & ~half->acted_peer;
	half->acted_peer |= peer;
	if (!fresh) {
		spin_unlock_irqrestore(&ctx->lock, flags);
		return;
	}
	if (fresh & SEND_SHUTDOWN) {
		if (fresh & RCV_SHUTDOWN) {
			if (item->watch) {
				watch_kind = END_WATCH_ABORTED;
				watch_cause = CAUSE_PEER;
				queue_cleanup = ovd_fwd_watch_end_locked(half, watch_kind,
									 watch_cause);
			} else if (item->end_kind == END_NONE) {
				half->source_end = SOURCE_ABORT;
				item->dir[half->side].source_end = SOURCE_ABORT;
				item->end_kind = END_ABORTED;
				item->end_cause = CAUSE_PEER;
				item->end_side = half->side;
				queue_direction = true;
			}
		} else {
			half->source_end = SOURCE_CLEAN;
			item->dir[half->side].source_end = SOURCE_CLEAN;
			if (item->watch) {
				watch_kind = END_WATCH_HALF_CLOSE;
				queue_cleanup = ovd_fwd_watch_end_locked(half, watch_kind,
									 CAUSE_NONE);
			} else {
				queue_direction = true;
			}
		}
	} else if (fresh & RCV_SHUTDOWN) {
		if (!(READ_ONCE(sk->sk_shutdown) & SEND_SHUTDOWN)) {
			if (item->watch) {
				watch_kind = END_WATCH_ABORTED;
				watch_cause = CAUSE_PEER;
				queue_cleanup = ovd_fwd_watch_end_locked(half, watch_kind,
									 watch_cause);
			} else if (item->end_kind == END_NONE) {
				half->source_end = SOURCE_ABORT;
				item->dir[half->side].source_end = SOURCE_ABORT;
				item->end_kind = END_ABORTED;
				item->end_cause = CAUSE_PEER;
				item->end_side = half->side;
				queue_direction = true;
			}
		}
	}
	spin_unlock_irqrestore(&ctx->lock, flags);
	if (item->watch && (watch_kind || item->releasing)) {
		if (queue_cleanup || READ_ONCE(item->releasing))
			ovd_fwd_kick(item, EV_FINISH);
	} else if (queue_direction) {
		ovd_fwd_kick(item, EV_DIR_A | EV_DIR_B);
	}
}

static void ovd_fwd_socket_error(struct ovd_fwd_half *half)
{
	struct ovd_fwd_item *item = half->item;
	int error = READ_ONCE(half->sk->sk_err);

	if (!error)
		return;
	if (item->watch) {
		ovd_fwd_record_watch(item, END_WATCH_ABORTED, CAUSE_SOCKET,
				     half->side, error);
		ovd_fwd_kick(item, EV_FINISH);
	} else {
		ovd_fwd_request_abort(item, CAUSE_SOCKET, half->side, error);
	}
}

static struct ovd_fwd_half *ovd_fwd_callback_half(struct sock *sk)
{
	/* Pair the callback-pointer restore with the release store to sk_user_data. */
	return smp_load_acquire(&sk->sk_user_data);
}

static void ovd_fwd_data_ready(struct sock *sk)
{
	struct ovd_fwd_half *half;
	void (*original)(struct sock *sk);

	rcu_read_lock();
	half = ovd_fwd_callback_half(sk);
	if (half && !half->item->watch) {
		ovd_fwd_kick(half->item, half->side == 0 ? EV_DIR_A : EV_DIR_B);
	} else {
		original = READ_ONCE(sk->sk_data_ready);
		if (original && original != ovd_fwd_data_ready)
			original(sk);
	}
	rcu_read_unlock();
}

static void ovd_fwd_state_change(struct sock *sk)
{
	struct ovd_fwd_half *half;
	void (*original)(struct sock *sk);

	rcu_read_lock();
	half = ovd_fwd_callback_half(sk);
	if (!half) {
		original = READ_ONCE(sk->sk_state_change);
		if (original && original != ovd_fwd_state_change)
			original(sk);
		rcu_read_unlock();
		return;
	}
	if (half->item->watch) {
		ovd_fwd_classify_vsock(half);
		if (half->orig_state_change)
			half->orig_state_change(sk);
	} else {
		if (half->orig_state_change)
			half->orig_state_change(sk);
		if (half->kind == OVD_KIND_VSOCK)
			ovd_fwd_classify_vsock(half);
		else
			ovd_fwd_kick(half->item, EV_DIR_A | EV_DIR_B);
	}
	rcu_read_unlock();
}

static void ovd_fwd_error_report(struct sock *sk)
{
	struct ovd_fwd_half *half;
	void (*original)(struct sock *sk);

	rcu_read_lock();
	half = ovd_fwd_callback_half(sk);
	if (!half) {
		original = READ_ONCE(sk->sk_error_report);
		if (original && original != ovd_fwd_error_report)
			original(sk);
		rcu_read_unlock();
		return;
	}
	if (half->item->watch) {
		ovd_fwd_socket_error(half);
		if (half->orig_error_report)
			half->orig_error_report(sk);
	} else {
		if (half->orig_error_report)
			half->orig_error_report(sk);
		ovd_fwd_socket_error(half);
		ovd_fwd_kick(half->item, EV_DIR_A | EV_DIR_B);
	}
	rcu_read_unlock();
}

static u8 ovd_fwd_kind(struct socket *sock)
{
	struct sock *sk = sock->sk;

	if (sk->sk_family == AF_VSOCK && sock->type == SOCK_STREAM)
		return OVD_KIND_VSOCK;
	if ((sk->sk_family == AF_INET || sk->sk_family == AF_INET6) &&
	    sock->type == SOCK_STREAM && sk->sk_protocol == IPPROTO_TCP)
		return OVD_KIND_TCP;
	if ((sk->sk_family == AF_INET || sk->sk_family == AF_INET6) &&
	    sock->type == SOCK_DGRAM && sk->sk_protocol == IPPROTO_UDP)
		return OVD_KIND_UDP;
	return OVD_KIND_OTHER;
}

static struct ovd_fwd_item *ovd_fwd_item_alloc(struct ovd_fwd_ctx *ctx,
					       bool watch, u64 id)
{
	struct ovd_fwd_item *item;

	item = kzalloc_obj(*item, GFP_KERNEL);
	if (!item)
		return NULL;
	refcount_set(&item->refs, 1);
	atomic_set(&item->work_refs, 0);
	atomic_long_set(&item->pending, 0);
	INIT_LIST_HEAD(&item->live_node);
	INIT_LIST_HEAD(&item->end_node);
	INIT_WORK(&item->work, ovd_fwd_work);
	INIT_RCU_WORK(&item->free_work, ovd_fwd_free_work);
	item->ctx = ctx;
	item->id = id;
	item->watch = watch;
	atomic_inc(&ctx->outstanding);
	return item;
}

static int ovd_fwd_ctx_id(struct ovd_fwd_ctx *ctx, u64 *id)
{
	unsigned long flags;

	spin_lock_irqsave(&ctx->lock, flags);
	if (ctx->releasing) {
		spin_unlock_irqrestore(&ctx->lock, flags);
		return -EBADF;
	}
	if (ctx->next_id == U64_MAX) {
		spin_unlock_irqrestore(&ctx->lock, flags);
		return -EOVERFLOW;
	}
	*id = ++ctx->next_id;
	spin_unlock_irqrestore(&ctx->lock, flags);
	return 0;
}

static int ovd_fwd_open_half(struct ovd_fwd_item *item, int fd, u8 side)
{
	struct ovd_fwd_half *half = &item->half[side];
	int error = 0;

	half->sock = sockfd_lookup(fd, &error);
	if (!half->sock)
		return error;
	half->sk = half->sock->sk;
	half->item = item;
	half->ctx = item->ctx;
	half->side = side;
	half->kind = ovd_fwd_kind(half->sock);
	return 0;
}

static int ovd_fwd_socket_flags(struct ovd_fwd_half *half)
{
	struct sock *sk = half->sk;

	if (half->kind == OVD_KIND_OTHER)
		return -ESOCKTNOSUPPORT;
	if (half->kind == OVD_KIND_TCP && inet_csk_has_ulp(sk))
		return -EPROTONOSUPPORT;
	if (sock_flag(sk, SOCK_ZEROCOPY) ||
	    (READ_ONCE(sk->sk_tsflags) & SOF_TIMESTAMPING_TX_RECORD_MASK))
		return -ESOCKTNOSUPPORT;
	return 0;
}

static int ovd_fwd_state_error(struct ovd_fwd_half *half, u16 *end_errno)
{
	struct sock *sk = half->sk;
	int state = READ_ONCE(sk->sk_state);
	int sk_error = READ_ONCE(sk->sk_err);

	*end_errno = 0;
	if (half->kind == OVD_KIND_UDP)
		return state == TCP_ESTABLISHED ? 0 : -ENOTCONN;
	if (sk_error) {
		*end_errno = (u16)sk_error;
		return -ECONNABORTED;
	}
	if (half->kind == OVD_KIND_VSOCK) {
		if (state == TCP_CLOSING) {
			*end_errno = ECONNRESET;
			return -ECONNABORTED;
		}
		if (state != TCP_ESTABLISHED ||
		    (READ_ONCE(sk->sk_shutdown) & SEND_SHUTDOWN))
			return -ENOTCONN;
		return 0;
	}
	if ((state == TCP_ESTABLISHED || state == TCP_CLOSE_WAIT) &&
	    !(READ_ONCE(sk->sk_shutdown) & SEND_SHUTDOWN))
		return 0;
	return -ENOTCONN;
}

static void ovd_fwd_seed_vsock(struct ovd_fwd_half *half)
{
	struct ovd_fwd_ctx *ctx = half->ctx;
	struct ovd_fwd_item *item = half->item;
	unsigned long flags;
	u8 peer = READ_ONCE(vsock_sk(half->sk)->peer_shutdown) & SHUTDOWN_MASK;

	half->acted_peer = peer;
	if (!(peer & SHUTDOWN_MASK))
		return;
	spin_lock_irqsave(&ctx->lock, flags);
	if (item->dead || item->end_kind != END_NONE || item->record_queued) {
		spin_unlock_irqrestore(&ctx->lock, flags);
		return;
	}
	if (peer == SEND_SHUTDOWN) {
		half->source_end = SOURCE_CLEAN;
		item->dir[half->side].source_end = SOURCE_CLEAN;
		if (item->watch) {
			item->end_kind = END_WATCH_HALF_CLOSE;
			item->end_cause = CAUSE_NONE;
		}
	} else if (peer & RCV_SHUTDOWN) {
		half->source_end = SOURCE_ABORT;
		item->dir[half->side].source_end = SOURCE_ABORT;
		if (item->watch) {
			item->end_kind = END_WATCH_ABORTED;
			item->end_cause = CAUSE_PEER;
			item->end_side = half->side;
		} else {
			item->end_kind = END_ABORTED;
			item->end_cause = CAUSE_PEER;
			item->end_side = half->side;
		}
	}
	spin_unlock_irqrestore(&ctx->lock, flags);
}

static int ovd_fwd_hook_half_locked(struct ovd_fwd_half *half, u16 *end_errno)
{
	struct sock *sk = half->sk;
	int error;

	error = ovd_fwd_socket_flags(half);
	if (error)
		return error;
	error = ovd_fwd_state_error(half, end_errno);
	if (error)
		return error;
	write_lock_bh(&sk->sk_callback_lock);
	if (READ_ONCE(sk->sk_user_data)) {
		write_unlock_bh(&sk->sk_callback_lock);
		return -EBUSY;
	}
	if (half->kind == OVD_KIND_VSOCK &&
	    half->item->mode != OVD_FWD_MODE_STREAM &&
	    READ_ONCE(vsock_sk(sk)->buffer_size) < OVD_FWD_FRAME_MAX) {
		write_unlock_bh(&sk->sk_callback_lock);
		return -EMSGSIZE;
	}
	half->orig_data_ready = sk->sk_data_ready;
	half->orig_state_change = sk->sk_state_change;
	half->orig_error_report = sk->sk_error_report;
	__module_get(THIS_MODULE);
	half->module_ref = true;
	if (!half->item->watch)
		WRITE_ONCE(sk->sk_data_ready, ovd_fwd_data_ready);
	WRITE_ONCE(sk->sk_state_change, ovd_fwd_state_change);
	WRITE_ONCE(sk->sk_error_report, ovd_fwd_error_report);
	/* Publish the installed callbacks before a reader can observe this half. */
	smp_store_release(&sk->sk_user_data, half);
	half->hooked = true;
	write_unlock_bh(&sk->sk_callback_lock);
	if (half->kind == OVD_KIND_VSOCK)
		ovd_fwd_seed_vsock(half);
	return 0;
}

static void ovd_fwd_unhook_half(struct ovd_fwd_half *half)
{
	struct sock *sk = half->sk;

	if (!half->hooked)
		return;
	lock_sock(sk);
	if (half->wait_added) {
		remove_wait_queue(sk_sleep(sk), &half->write_wait);
		half->wait_added = false;
	}
	write_lock_bh(&sk->sk_callback_lock);
	if (!half->item->watch)
		WRITE_ONCE(sk->sk_data_ready, half->orig_data_ready);
	WRITE_ONCE(sk->sk_state_change, half->orig_state_change);
	WRITE_ONCE(sk->sk_error_report, half->orig_error_report);
	/* Restore callbacks before a reader can observe that this half is unhooked. */
	smp_store_release(&sk->sk_user_data, NULL);
	half->hooked = false;
	write_unlock_bh(&sk->sk_callback_lock);
	if (half->kind == OVD_KIND_TCP && inet_csk_has_ulp(sk))
		half->keep_module = true;
	release_sock(sk);
}

static int ovd_fwd_validate_pair_kinds(struct ovd_fwd_item *item, u32 mode,
				       u8 *bad_side)
{
	u8 a = item->half[0].kind;
	u8 b = item->half[1].kind;

	if (mode == OVD_FWD_MODE_STREAM) {
		if (a == OVD_KIND_VSOCK && b == OVD_KIND_TCP)
			return 0;
		if (a == OVD_KIND_TCP && b == OVD_KIND_VSOCK)
			return 0;
		if (a == OVD_KIND_VSOCK)
			*bad_side = 1;
		else if (b == OVD_KIND_VSOCK)
			*bad_side = 0;
		else if (a == OVD_KIND_OTHER)
			*bad_side = 0;
		else if (b == OVD_KIND_OTHER)
			*bad_side = 1;
		else
			*bad_side = 0;
		return -ESOCKTNOSUPPORT;
	}
	if (mode == OVD_FWD_MODE_DGRAM_UDP_A) {
		if (a != OVD_KIND_UDP) {
			*bad_side = 0;
			return -ESOCKTNOSUPPORT;
		}
		if (b != OVD_KIND_VSOCK) {
			*bad_side = 1;
			return -ESOCKTNOSUPPORT;
		}
		return 0;
	}
	if (mode == OVD_FWD_MODE_DGRAM_UDP_B) {
		if (a != OVD_KIND_VSOCK) {
			*bad_side = 0;
			return -ESOCKTNOSUPPORT;
		}
		if (b != OVD_KIND_UDP) {
			*bad_side = 1;
			return -ESOCKTNOSUPPORT;
		}
		return 0;
	}
	*bad_side = 0;
	return -EINVAL;
}

static int ovd_fwd_validate_flags(struct ovd_fwd_item *item, int n,
				  u8 *bad_side)
{
	int side;

	for (side = 0; side < n; side++) {
		struct ovd_fwd_half *half = &item->half[side];
		bool unsupported;

		lock_sock(half->sk);
		unsupported = sock_flag(half->sk, SOCK_ZEROCOPY) ||
			(READ_ONCE(half->sk->sk_tsflags) &
			 SOF_TIMESTAMPING_TX_RECORD_MASK);
		release_sock(half->sk);
		if (unsupported) {
			*bad_side = side;
			return -ESOCKTNOSUPPORT;
		}
	}
	for (side = 0; side < n; side++) {
		struct ovd_fwd_half *half = &item->half[side];
		bool has_ulp = false;

		if (half->kind == OVD_KIND_TCP) {
			lock_sock(half->sk);
			has_ulp = inet_csk_has_ulp(half->sk);
			release_sock(half->sk);
		}
		if (has_ulp) {
			*bad_side = side;
			return -EPROTONOSUPPORT;
		}
	}
	return 0;
}

static int ovd_fwd_validate_states(struct ovd_fwd_item *item, int n,
				   u8 *bad_side, u16 *end_errno)
{
	int side;
	int error;

	for (side = 0; side < n; side++) {
		struct ovd_fwd_half *half = &item->half[side];

		lock_sock(half->sk);
		if (half->kind == OVD_KIND_VSOCK)
			spin_lock_bh(&vsock_table_lock);
		error = ovd_fwd_state_error(half, end_errno);
		if (half->kind == OVD_KIND_VSOCK)
			spin_unlock_bh(&vsock_table_lock);
		release_sock(half->sk);
		if (error) {
			*bad_side = side;
			return error;
		}
	}
	*end_errno = 0;
	return 0;
}

static int ovd_fwd_busy_side(struct ovd_fwd_item *item, int n, u8 *bad_side)
{
	int side;

	for (side = 0; side < n; side++) {
		struct ovd_fwd_half *half = &item->half[side];
		bool busy;

		read_lock_bh(&half->sk->sk_callback_lock);
		busy = !!READ_ONCE(half->sk->sk_user_data);
		read_unlock_bh(&half->sk->sk_callback_lock);
		if (busy) {
			*bad_side = side;
			return -EBUSY;
		}
	}
	return 0;
}

static int ovd_fwd_attach_half(struct ovd_fwd_half *half, u16 *end_errno)
{
	struct sock *sk = half->sk;
	int error;

	lock_sock(sk);
	if (half->kind == OVD_KIND_VSOCK)
		spin_lock_bh(&vsock_table_lock);
	error = ovd_fwd_hook_half_locked(half, end_errno);
	if (half->kind == OVD_KIND_VSOCK)
		spin_unlock_bh(&vsock_table_lock);
	if (!error && !half->item->watch) {
		init_waitqueue_func_entry(&half->write_wait, ovd_fwd_write_wake);
		add_wait_queue(sk_sleep(sk), &half->write_wait);
		half->wait_added = true;
	}
	release_sock(sk);
	if (error)
		return error;
	return 0;
}

static void ovd_fwd_discard_item(struct ovd_fwd_item *item)
{
	int side;
	unsigned int module_puts = 0;

	for (side = 0; side < 2; side++)
		ovd_fwd_unhook_half(&item->half[side]);
	synchronize_rcu();
	for (side = 0; side < 2; side++) {
		struct ovd_fwd_half *half = &item->half[side];

		if (half->sock) {
			sockfd_put(half->sock);
			half->sock = NULL;
		}
		if (half->module_ref && !half->keep_module)
			module_puts++;
	}
	ovd_fwd_item_put(item);
	while (module_puts--)
		module_put(THIS_MODULE);
}

static void ovd_fwd_register_item(struct ovd_fwd_item *item)
{
	struct ovd_fwd_ctx *ctx = item->ctx;
	unsigned long flags;
	bool watch_end = false;

	spin_lock_irqsave(&ctx->lock, flags);
	list_add_tail(&item->live_node, &ctx->live);
	if (item->watch) {
		ctx->live_watches++;
		watch_end = item->end_kind == END_WATCH_HALF_CLOSE ||
			item->end_kind == END_WATCH_ABORTED;
	} else {
		ctx->live_pairs++;
		atomic64_inc(&ctx->attached);
	}
	WRITE_ONCE(item->ready, true);
	if (watch_end) {
		ovd_fwd_record_watch_locked(item, item->end_kind, item->end_cause,
					    item->end_side, item->end_errno);
	} else if (!item->watch) {
		ovd_fwd_kick(item, item->end_kind == END_ABORTED ?
				      EV_FINISH : (EV_DIR_A | EV_DIR_B));
	}
	spin_unlock_irqrestore(&ctx->lock, flags);
	wake_up_all(&ctx->wait);
	if (watch_end)
		wake_up_interruptible(&ctx->wait);
}

static long ovd_fwd_refusal(struct ovd_fwd_attach __user *user,
			    struct ovd_fwd_attach *request, int error,
			    u8 bad_side, u16 end_errno)
{
	request->bad_side = bad_side;
	request->end_errno = error == -ECONNABORTED ? end_errno : 0;
	if (copy_to_user(user, request, sizeof(*request)))
		return -EFAULT;
	return error;
}

static long ovd_fwd_attach(struct ovd_fwd_ctx *ctx,
			   struct ovd_fwd_attach __user *user)
{
	struct ovd_fwd_attach request;
	struct ovd_fwd_item *item;
	u64 id;
	u16 end_errno = 0;
	u8 bad_side = 0;
	int error;
	int side;
	int n = 2;
	struct ovd_fwd_half *vsock;

	if (copy_from_user(&request, user, sizeof(request)))
		return -EFAULT;
	mutex_lock(&ctx->ops_lock);
	error = ovd_fwd_ctx_id(ctx, &id);
	if (error)
		goto out_unlock;
	item = ovd_fwd_item_alloc(ctx, false, id);
	if (!item) {
		error = -ENOMEM;
		goto out_unlock;
	}
	item->mode = request.mode;
	for (side = 0; side < 2; side++) {
		int fd = side == 0 ? request.fd_a : request.fd_b;

		bad_side = side;
		error = ovd_fwd_open_half(item, fd, side);
		if (error)
			goto refuse;
	}
	if (item->half[0].sk == item->half[1].sk) {
		bad_side = 1;
		error = -EINVAL;
		goto refuse;
	}
	if (request.flags || request.mode > OVD_FWD_MODE_DGRAM_UDP_B) {
		bad_side = 0;
		error = -EINVAL;
		goto refuse;
	}
	error = ovd_fwd_validate_pair_kinds(item, request.mode, &bad_side);
	if (error)
		goto refuse;
	error = ovd_fwd_validate_flags(item, n, &bad_side);
	if (error)
		goto refuse;
	error = ovd_fwd_validate_states(item, n, &bad_side, &end_errno);
	if (error)
		goto refuse;
	error = ovd_fwd_busy_side(item, n, &bad_side);
	if (error)
		goto refuse;
	if (request.mode != OVD_FWD_MODE_STREAM) {
		vsock = item->half[0].kind == OVD_KIND_VSOCK ?
			&item->half[0] : &item->half[1];
		if (READ_ONCE(vsock_sk(vsock->sk)->buffer_size) <
		    OVD_FWD_FRAME_MAX) {
			bad_side = vsock->side;
			error = -EMSGSIZE;
			goto refuse;
		}
	}
	for (side = 0; side < n; side++) {
		bad_side = side;
		end_errno = 0;
		error = ovd_fwd_attach_half(&item->half[side], &end_errno);
		if (error)
			goto refuse;
	}
	request.id = id;
	request.bad_side = 0;
	request.end_errno = 0;
	if (copy_to_user(user, &request, sizeof(request))) {
		error = -EFAULT;
		goto refuse_no_result;
	}
	ovd_fwd_register_item(item);
	mutex_unlock(&ctx->ops_lock);
	return 0;

refuse:
	error = ovd_fwd_refusal(user, &request, error, bad_side, end_errno);
refuse_no_result:
	ovd_fwd_discard_item(item);
out_unlock:
	mutex_unlock(&ctx->ops_lock);
	return error;
}

static long ovd_fwd_watch(struct ovd_fwd_ctx *ctx,
			  struct ovd_fwd_watch __user *user)
{
	struct ovd_fwd_watch request;
	struct ovd_fwd_item *item;
	u64 id;
	u16 end_errno = 0;
	u8 bad_side = 0;
	int error;

	if (copy_from_user(&request, user, sizeof(request)))
		return -EFAULT;
	mutex_lock(&ctx->ops_lock);
	error = ovd_fwd_ctx_id(ctx, &id);
	if (error)
		goto out_unlock;
	item = ovd_fwd_item_alloc(ctx, true, id);
	if (!item) {
		error = -ENOMEM;
		goto out_unlock;
	}
	error = ovd_fwd_open_half(item, request.fd, 0);
	if (error)
		goto refuse;
	if (request.flags || item->half[0].kind != OVD_KIND_VSOCK) {
		error = request.flags ? -EINVAL : -ESOCKTNOSUPPORT;
		goto refuse;
	}
	error = ovd_fwd_validate_flags(item, 1, &bad_side);
	if (error)
		goto refuse;
	error = ovd_fwd_validate_states(item, 1, &bad_side, &end_errno);
	if (error)
		goto refuse;
	error = ovd_fwd_busy_side(item, 1, &bad_side);
	if (error)
		goto refuse;
	error = ovd_fwd_attach_half(&item->half[0], &end_errno);
	if (error)
		goto refuse;
	request.id = id;
	request.bad_side = 0;
	request.end_errno = 0;
	if (copy_to_user(user, &request, sizeof(request))) {
		error = -EFAULT;
		goto refuse_no_result;
	}
	ovd_fwd_register_item(item);
	mutex_unlock(&ctx->ops_lock);
	return 0;

refuse:
	request.bad_side = bad_side;
	request.end_errno = error == -ECONNABORTED ? end_errno : 0;
	if (copy_to_user(user, &request, sizeof(request)))
		error = -EFAULT;
refuse_no_result:
	ovd_fwd_discard_item(item);
out_unlock:
	mutex_unlock(&ctx->ops_lock);
	return error;
}

static bool ovd_fwd_live_empty(struct ovd_fwd_ctx *ctx)
{
	unsigned long flags;
	bool empty;

	spin_lock_irqsave(&ctx->lock, flags);
	empty = list_empty(&ctx->live);
	spin_unlock_irqrestore(&ctx->lock, flags);
	return empty;
}

static u32 ovd_fwd_end_all(struct ovd_fwd_ctx *ctx, bool release)
{
	struct ovd_fwd_item *item;
	struct ovd_fwd_item *next;
	unsigned long flags;
	u32 count = 0;

	mutex_lock(&ctx->ops_lock);
	spin_lock_irqsave(&ctx->lock, flags);
	if (release)
		ctx->releasing = true;
	list_for_each_entry_safe(item, next, &ctx->live, live_node) {
		count++;
		if (release)
			item->releasing = true;
		if (item->watch) {
			if (release) {
				if (item->end_kind == END_NONE) {
					item->end_kind = END_WATCH_ABORTED;
					item->end_cause = CAUSE_DETACHED;
				}
				ovd_fwd_kick(item, EV_FINISH);
			} else if (item->end_kind == END_NONE) {
				ovd_fwd_record_watch_locked(item, END_WATCH_ABORTED,
							    CAUSE_DETACHED, 0, 0);
			} else {
				ovd_fwd_kick(item, EV_FINISH);
			}
		} else {
			if (item->end_kind == END_NONE) {
				item->end_kind = END_ABORTED;
				item->end_cause = CAUSE_DETACHED;
			}
			ovd_fwd_kick(item, EV_FINISH);
		}
	}
	spin_unlock_irqrestore(&ctx->lock, flags);
	wake_up_all(&ctx->wait);
	if (release) {
		struct ovd_fwd_item *record;
		struct ovd_fwd_item *record_next;
		LIST_HEAD(discard);

		spin_lock_irqsave(&ctx->lock, flags);
		list_splice_init(&ctx->ends, &discard);
		ctx->queued_ends = 0;
		spin_unlock_irqrestore(&ctx->lock, flags);
		list_for_each_entry_safe(record, record_next, &discard, end_node) {
			list_del_init(&record->end_node);
			ovd_fwd_item_put(record);
		}
		wait_event(ctx->wait,
			   ovd_fwd_live_empty(ctx) &&
			   atomic_read(&ctx->outstanding) == 0 &&
			   atomic_read(&ctx->cleanup_pending) == 0);
	} else {
		wait_event(ctx->wait,
			   ovd_fwd_live_empty(ctx) &&
			   atomic_read(&ctx->cleanup_pending) == 0);
	}
	mutex_unlock(&ctx->ops_lock);
	return count;
}

static long ovd_fwd_read(struct file *file, char __user *user_buffer,
			 size_t length, loff_t *offset)
{
	struct ovd_fwd_ctx *ctx = file->private_data;
	size_t done = 0;
	long result;

	(void)offset;
	if (length < sizeof(struct ovd_fwd_end))
		return -EINVAL;
	mutex_lock(&ctx->read_lock);
	for (;;) {
		struct ovd_fwd_item *item;
		struct ovd_fwd_end record;
		unsigned long flags;

		spin_lock_irqsave(&ctx->lock, flags);
		if (list_empty(&ctx->ends)) {
			bool releasing = ctx->releasing;

			spin_unlock_irqrestore(&ctx->lock, flags);
			if (done || releasing) {
				result = done;
				break;
			}
			if (file->f_flags & O_NONBLOCK) {
				result = -EAGAIN;
				break;
			}
			result = wait_event_interruptible(ctx->wait,
							  READ_ONCE(ctx->queued_ends) ||
							  READ_ONCE(ctx->releasing));
			if (result)
				break;
			continue;
		}
		item = list_first_entry(&ctx->ends, struct ovd_fwd_item, end_node);
		record = item->record;
		spin_unlock_irqrestore(&ctx->lock, flags);

		if (copy_to_user(user_buffer + done, &record, sizeof(record))) {
			result = done ? done : -EFAULT;
			break;
		}
		spin_lock_irqsave(&ctx->lock, flags);
		if (!list_empty(&item->end_node)) {
			list_del_init(&item->end_node);
			ctx->queued_ends--;
		}
		spin_unlock_irqrestore(&ctx->lock, flags);
		done += sizeof(record);
		ovd_fwd_item_put(item);
		if (done + sizeof(record) > length) {
			result = done;
			break;
		}
	}
	mutex_unlock(&ctx->read_lock);
	return result;
}

static __poll_t ovd_fwd_poll(struct file *file, poll_table *wait)
{
	struct ovd_fwd_ctx *ctx = file->private_data;
	__poll_t result = 0;

	poll_wait(file, &ctx->wait, wait);
	if (READ_ONCE(ctx->queued_ends))
		result = EPOLLIN | EPOLLRDNORM;
	return result;
}

static long ovd_fwd_ioctl(struct file *file, unsigned int command,
			  unsigned long argument)
{
	struct ovd_fwd_ctx *ctx = file->private_data;
	void __user *user = (void __user *)argument;

	switch (command) {
	case OVD_FWD_VERSION: {
		struct ovd_fwd_version version = {
			.abi_version = OVD_FWD_ABI_VERSION,
		};

		return copy_to_user(user, &version, sizeof(version)) ? -EFAULT : 0;
	}
	case OVD_FWD_ATTACH:
		return ovd_fwd_attach(ctx, user);
	case OVD_FWD_WATCH:
		return ovd_fwd_watch(ctx, user);
	case OVD_FWD_ABORT: {
		u64 id;

		if (copy_from_user(&id, user, sizeof(id)))
			return -EFAULT;
		return ovd_fwd_abort_id(ctx, id);
	}
	case OVD_FWD_ABORT_ALL:
		return ovd_fwd_end_all(ctx, false);
	case OVD_FWD_INVENTORY:
		return ovd_fwd_inventory(ctx, user);
	case OVD_FWD_PAIR_STATE:
		return ovd_fwd_pair_state(ctx, user);
	default:
		return -ENOTTY;
	}
}

static int ovd_fwd_open(struct inode *inode, struct file *file)
{
	struct ovd_fwd_ctx *ctx;
	int error;

	ctx = kzalloc_obj(*ctx, GFP_KERNEL);
	if (!ctx)
		return -ENOMEM;
	spin_lock_init(&ctx->lock);
	mutex_init(&ctx->ops_lock);
	mutex_init(&ctx->read_lock);
	INIT_LIST_HEAD(&ctx->live);
	INIT_LIST_HEAD(&ctx->ends);
	init_waitqueue_head(&ctx->wait);
	atomic_set(&ctx->outstanding, 0);
	atomic_set(&ctx->cleanup_pending, 0);
	file->private_data = ctx;
	error = nonseekable_open(inode, file);
	if (error)
		kfree(ctx);
	return error;
}

static int ovd_fwd_release(struct inode *inode, struct file *file)
{
	struct ovd_fwd_ctx *ctx = file->private_data;

	(void)inode;
	ovd_fwd_end_all(ctx, true);
	kfree(ctx);
	return 0;
}

static const struct file_operations ovd_fwd_fops = {
	.owner = THIS_MODULE,
	.open = ovd_fwd_open,
	.release = ovd_fwd_release,
	.read = ovd_fwd_read,
	.poll = ovd_fwd_poll,
	.unlocked_ioctl = ovd_fwd_ioctl,
	.compat_ioctl = ovd_fwd_ioctl,
};

static struct miscdevice ovd_fwd_device = {
	.minor = MISC_DYNAMIC_MINOR,
	.name = "ovd_fwd",
	.fops = &ovd_fwd_fops,
	.mode = 0600,
};

static int __init ovd_fwd_init(void)
{
	int error;

	ovd_fwd_workqueue = alloc_workqueue("ovd_fwd", WQ_PERCPU | WQ_MEM_RECLAIM, 0);
	if (!ovd_fwd_workqueue)
		return -ENOMEM;
	ovd_fwd_scratch_pool = mempool_create_kmalloc_pool(FWD_SCRATCH_POOL,
							   FWD_CHUNK);
	if (!ovd_fwd_scratch_pool) {
		destroy_workqueue(ovd_fwd_workqueue);
		ovd_fwd_workqueue = NULL;
		return -ENOMEM;
	}
	error = misc_register(&ovd_fwd_device);
	if (error) {
		mempool_destroy(ovd_fwd_scratch_pool);
		ovd_fwd_scratch_pool = NULL;
		destroy_workqueue(ovd_fwd_workqueue);
		ovd_fwd_workqueue = NULL;
	}
	return error;
}

static void __exit ovd_fwd_exit(void)
{
	misc_deregister(&ovd_fwd_device);
	destroy_workqueue(ovd_fwd_workqueue);
	mempool_destroy(ovd_fwd_scratch_pool);
}

module_init(ovd_fwd_init);
module_exit(ovd_fwd_exit);

MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("Overdrive event-driven flow forwarder");
MODULE_VERSION(OVD_FWD_MODULE_VERSION);

enum ovd_fwd_step {
	OVD_STEP_BLOCKED,
	OVD_STEP_MORE,
	OVD_STEP_ENDED,
};

static int ovd_fwd_consume(struct ovd_fwd_half *half, size_t length)
{
	struct msghdr message = { .msg_flags = MSG_DONTWAIT };
	int flags = MSG_DONTWAIT;
	int result;

	if (half->kind == OVD_KIND_UDP)
		flags |= MSG_TRUNC;
	iov_iter_discard(&message.msg_iter, ITER_DEST, length);
	result = sock_recvmsg(half->sock, &message, flags);
	if (result < 0)
		return result;
	if ((size_t)result != length)
		return -EIO;
	return 0;
}

static int ovd_fwd_send(struct ovd_fwd_half *half, const void *bytes,
			size_t length)
{
	struct msghdr message = { .msg_flags = MSG_DONTWAIT | MSG_NOSIGNAL };
	struct kvec vector = { .iov_base = (void *)bytes, .iov_len = length };

	return kernel_sendmsg(half->sock, &message, &vector, 1, length);
}

static void ovd_fwd_mark_source_end(struct ovd_fwd_item *item, u8 source,
				    u8 source_end)
{
	unsigned long flags;

	spin_lock_irqsave(&item->ctx->lock, flags);
	item->half[source].source_end = source_end;
	item->dir[source].source_end = source_end;
	spin_unlock_irqrestore(&item->ctx->lock, flags);
}

static bool ovd_fwd_target_shut(struct ovd_fwd_item *item, u8 source)
{
	unsigned long flags;
	bool target_shut;

	spin_lock_irqsave(&item->ctx->lock, flags);
	target_shut = item->dir[source].target_shut;
	spin_unlock_irqrestore(&item->ctx->lock, flags);
	return target_shut;
}

static u8 ovd_fwd_source_end(struct ovd_fwd_item *item, u8 source)
{
	unsigned long flags;
	u8 source_end;

	spin_lock_irqsave(&item->ctx->lock, flags);
	source_end = item->half[source].source_end;
	spin_unlock_irqrestore(&item->ctx->lock, flags);
	return source_end;
}

static bool ovd_fwd_pair_aborted(struct ovd_fwd_item *item)
{
	unsigned long flags;
	bool aborted;

	spin_lock_irqsave(&item->ctx->lock, flags);
	aborted = item->end_kind == END_ABORTED || item->releasing;
	spin_unlock_irqrestore(&item->ctx->lock, flags);
	return aborted;
}

static int ovd_fwd_vsock_end_after_recheck(struct ovd_fwd_half *half)
{
	struct ovd_fwd_item *item = half->item;
	struct ovd_fwd_ctx *ctx = item->ctx;
	unsigned long flags;
	int error = 0;

	spin_lock_bh(&vsock_table_lock);
	spin_lock_irqsave(&ctx->lock, flags);
	if (item->end_kind == END_ABORTED || half->source_end == SOURCE_ABORT) {
		error = 1;
	} else if (half->source_end == SOURCE_CLEAN) {
		error = 0;
	} else if (READ_ONCE(half->sk->sk_err)) {
		item->end_kind = END_ABORTED;
		item->end_cause = CAUSE_SOCKET;
		item->end_side = half->side;
		item->end_errno = READ_ONCE(half->sk->sk_err);
		half->source_end = SOURCE_ABORT;
		item->dir[half->side].source_end = SOURCE_ABORT;
		error = 1;
	} else {
		item->end_kind = END_ABORTED;
		item->end_cause = CAUSE_UNCLASSIFIED;
		item->end_side = half->side;
		half->source_end = SOURCE_ABORT;
		item->dir[half->side].source_end = SOURCE_ABORT;
		error = 1;
	}
	spin_unlock_irqrestore(&ctx->lock, flags);
	spin_unlock_bh(&vsock_table_lock);
	return error;
}

static int ovd_fwd_stream_step(struct ovd_fwd_item *item, u8 source,
			       void *scratch, int *budget)
{
	struct ovd_fwd_half *src = &item->half[source];
	struct ovd_fwd_half *dst = &item->half[!source];
	struct msghdr message = { .msg_flags = MSG_DONTWAIT };
	struct kvec vector = { .iov_base = scratch, .iov_len = FWD_CHUNK };
	int result;
	int sent;
	int error;

	if (ovd_fwd_pair_aborted(item))
		return OVD_STEP_ENDED;
	result = kernel_recvmsg(src->sock, &message, &vector, 1, FWD_CHUNK,
				MSG_PEEK | MSG_DONTWAIT);
	if (result == -EAGAIN || result == -EWOULDBLOCK)
		return OVD_STEP_BLOCKED;
	if (result < 0) {
		if (src->kind == OVD_KIND_VSOCK &&
		    ovd_fwd_vsock_end_after_recheck(src))
			return OVD_STEP_ENDED;
		ovd_fwd_request_abort(item, CAUSE_SOCKET, source, -result);
		return OVD_STEP_ENDED;
	}
	if (result == 0) {
		if (src->kind == OVD_KIND_VSOCK) {
			if (ovd_fwd_vsock_end_after_recheck(src))
				return OVD_STEP_ENDED;
		} else if (ovd_fwd_source_end(item, source) == SOURCE_NONE) {
			ovd_fwd_mark_source_end(item, source, SOURCE_CLEAN);
		}
		if (ovd_fwd_source_end(item, source) == SOURCE_CLEAN) {
			if (!ovd_fwd_target_shut(item, source)) {
				error = kernel_sock_shutdown(dst->sock, SHUT_WR);
				if (error < 0) {
					ovd_fwd_request_abort(item, CAUSE_SOCKET, !source,
							      -error);
					return OVD_STEP_ENDED;
				}
				spin_lock_bh(&item->ctx->lock);
				item->dir[source].target_shut = true;
				spin_unlock_bh(&item->ctx->lock);
			}
			return OVD_STEP_ENDED;
		}
		return OVD_STEP_BLOCKED;
	}
	sent = ovd_fwd_send(dst, scratch, result);
	if (sent == -EAGAIN || sent == -EWOULDBLOCK)
		return OVD_STEP_BLOCKED;
	if (sent < 0) {
		ovd_fwd_request_abort(item, CAUSE_SOCKET, !source, -sent);
		return OVD_STEP_ENDED;
	}
	if (!sent)
		return OVD_STEP_BLOCKED;
	error = ovd_fwd_consume(src, sent);
	if (error) {
		ovd_fwd_request_abort(item, CAUSE_SOCKET, source, -error);
		return OVD_STEP_ENDED;
	}
	spin_lock_bh(&item->ctx->lock);
	item->dir[source].moved += sent;
	spin_unlock_bh(&item->ctx->lock);
	*budget -= sent;
	if (sent < result)
		return OVD_STEP_BLOCKED;
	return *budget <= 0 ? OVD_STEP_MORE : OVD_STEP_MORE;
}

static int ovd_fwd_datagram_step(struct ovd_fwd_item *item, u8 source,
				 void *scratch, int *budget)
{
	struct ovd_fwd_half *src = &item->half[source];
	struct ovd_fwd_half *dst = &item->half[!source];
	struct ovd_fwd_direction *direction = &item->dir[source];
	struct msghdr message = { .msg_flags = MSG_DONTWAIT };
	struct kvec vector = { .iov_base = scratch, .iov_len = FWD_CHUNK };
	int result;
	int sent;
	int error;

	if (ovd_fwd_pair_aborted(item))
		return OVD_STEP_ENDED;
	if (src->kind == OVD_KIND_UDP) {
		result = kernel_recvmsg(src->sock, &message, &vector, 1, FWD_CHUNK,
					MSG_PEEK | MSG_DONTWAIT | MSG_TRUNC);
		if (result == -EAGAIN || result == -EWOULDBLOCK)
			return OVD_STEP_BLOCKED;
		if (result < 0) {
			ovd_fwd_request_abort(item, CAUSE_SOCKET, source, -result);
			return OVD_STEP_ENDED;
		}
		if (result > OVD_FWD_FRAME_MAX_PAYLOAD) {
			error = ovd_fwd_consume(src, result);
			if (error) {
				ovd_fwd_request_abort(item, CAUSE_SOCKET, source, -error);
				return OVD_STEP_ENDED;
			}
			atomic64_inc(&item->ctx->dgram_oversize);
			return OVD_STEP_MORE;
		}
		if (direction->dgram_started &&
		    direction->dgram_need != OVD_FWD_FRAME_HDR_LEN + result)
			return -EIO;
		memmove((u8 *)scratch + OVD_FWD_FRAME_HDR_LEN, scratch, result);
		((u8 *)scratch)[0] = 'O';
		((u8 *)scratch)[1] = 'V';
		((u8 *)scratch)[2] = 'D';
		((u8 *)scratch)[3] = '1';
		((u8 *)scratch)[4] = (__u8)((__u32)result >> 24);
		((u8 *)scratch)[5] = (__u8)((__u32)result >> 16);
		((u8 *)scratch)[6] = (__u8)((__u32)result >> 8);
		((u8 *)scratch)[7] = (__u8)result;
		if (!direction->dgram_started) {
			direction->dgram_started = true;
			direction->dgram_need = OVD_FWD_FRAME_HDR_LEN + result;
		}
		if (direction->dgram_sent > direction->dgram_need)
			return -EIO;
		sent = ovd_fwd_send(dst,
				    (u8 *)scratch + direction->dgram_sent,
				    direction->dgram_need - direction->dgram_sent);
		if (sent == -EAGAIN || sent == -EWOULDBLOCK)
			return OVD_STEP_BLOCKED;
		if (sent < 0) {
			ovd_fwd_request_abort(item, CAUSE_SOCKET, !source, -sent);
			return OVD_STEP_ENDED;
		}
		direction->dgram_sent += sent;
		*budget -= sent;
		if (direction->dgram_sent < direction->dgram_need)
			return OVD_STEP_BLOCKED;
		error = ovd_fwd_consume(src, result);
		if (error) {
			ovd_fwd_request_abort(item, CAUSE_SOCKET, source, -error);
			return OVD_STEP_ENDED;
		}
		spin_lock_bh(&item->ctx->lock);
		direction->moved += result;
		spin_unlock_bh(&item->ctx->lock);
		direction->dgram_started = false;
		direction->dgram_sent = 0;
		direction->dgram_need = 0;
		return OVD_STEP_MORE;
	}
	if (src->kind != OVD_KIND_VSOCK || dst->kind != OVD_KIND_UDP)
		return -ESOCKTNOSUPPORT;
	{
		s64 queued = vsock_stream_has_data(vsock_sk(src->sk));
		__u32 need = 0;
		__u32 payload_len = 0;
		__u32 header_len;
		int parse;

		if (queued < 0)
			return (int)queued;
		if (!queued) {
			if (ovd_fwd_source_end(item, source) == SOURCE_NONE &&
			    ((READ_ONCE(vsock_sk(src->sk)->peer_shutdown) & SHUTDOWN_MASK) ||
			     READ_ONCE(src->sk->sk_err) ||
			     READ_ONCE(src->sk->sk_state) != TCP_ESTABLISHED) &&
			    ovd_fwd_vsock_end_after_recheck(src))
				return OVD_STEP_ENDED;
			if (ovd_fwd_source_end(item, source) == SOURCE_CLEAN) {
				ovd_fwd_request_complete(item);
				return OVD_STEP_ENDED;
			}
			return OVD_STEP_BLOCKED;
		}
		header_len = min_t(s64, queued, OVD_FWD_FRAME_HDR_LEN);
		result = kernel_recvmsg(src->sock, &message, &vector, 1, header_len,
					MSG_PEEK | MSG_DONTWAIT);
		if (result == -EAGAIN || result == -EWOULDBLOCK)
			return OVD_STEP_BLOCKED;
		if (result < 0) {
			if (ovd_fwd_vsock_end_after_recheck(src))
				return OVD_STEP_ENDED;
			ovd_fwd_request_abort(item, CAUSE_SOCKET, source, -result);
			return OVD_STEP_ENDED;
		}
		parse = ovd_fwd_frame_parse(scratch, result, queued, &need, &payload_len);
		if (parse == OVD_FWD_FRAME_INVALID || parse == OVD_FWD_FRAME_OVERSIZE) {
			ovd_fwd_request_abort(item, CAUSE_FRAME, source, 0);
			return OVD_STEP_ENDED;
		}
		if (parse == OVD_FWD_FRAME_NEED_MORE) {
			direction->dgram_need = need;
			if (ovd_fwd_source_end(item, source) == SOURCE_CLEAN) {
				ovd_fwd_request_abort(item, CAUSE_FRAME, source, 0);
				return OVD_STEP_ENDED;
			}
			return OVD_STEP_BLOCKED;
		}
		result = kernel_recvmsg(src->sock, &message, &vector, 1, need,
					MSG_PEEK | MSG_DONTWAIT);
		if (result == -EAGAIN || result == -EWOULDBLOCK)
			return OVD_STEP_BLOCKED;
		if (result < 0) {
			if (ovd_fwd_vsock_end_after_recheck(src))
				return OVD_STEP_ENDED;
			ovd_fwd_request_abort(item, CAUSE_SOCKET, source, -result);
			return OVD_STEP_ENDED;
		}
		if (result != need) {
			ovd_fwd_request_abort(item, CAUSE_FRAME, source, 0);
			return OVD_STEP_ENDED;
		}
		sent = ovd_fwd_send(dst, (u8 *)scratch + OVD_FWD_FRAME_HDR_LEN,
				    payload_len);
		if (sent == -EAGAIN || sent == -EWOULDBLOCK)
			return OVD_STEP_BLOCKED;
		if (sent < 0) {
			ovd_fwd_request_abort(item, CAUSE_SOCKET, !source, -sent);
			return OVD_STEP_ENDED;
		}
		if ((__u32)sent != payload_len) {
			ovd_fwd_request_abort(item, CAUSE_SOCKET, !source, EIO);
			return OVD_STEP_ENDED;
		}
		error = ovd_fwd_consume(src, need);
		if (error) {
			ovd_fwd_request_abort(item, CAUSE_SOCKET, source, -error);
			return OVD_STEP_ENDED;
		}
		spin_lock_bh(&item->ctx->lock);
		direction->moved += payload_len;
		spin_unlock_bh(&item->ctx->lock);
		*budget -= need;
		return OVD_STEP_MORE;
	}
}

static bool ovd_fwd_pair_record(struct ovd_fwd_item *item)
{
	struct ovd_fwd_ctx *ctx = item->ctx;
	unsigned long flags;
	bool releasing;
	bool queued = false;

	spin_lock_irqsave(&ctx->lock, flags);
	if (!list_empty(&item->live_node)) {
		list_del_init(&item->live_node);
		ctx->live_pairs--;
	}
	releasing = item->releasing || ctx->releasing;
	if (releasing) {
		item->releasing = true;
	} else if (!item->record_queued) {
		ovd_fwd_record_fill(item);
		item->record_queued = true;
		list_add_tail(&item->end_node, &ctx->ends);
		ctx->queued_ends++;
		ovd_fwd_counter_end(ctx, item->end_kind, item->end_cause);
		queued = true;
	}
	spin_unlock_irqrestore(&ctx->lock, flags);
	wake_up_all(&ctx->wait);
	if (!releasing)
		wake_up_interruptible(&ctx->wait);
	return queued;
}

static void ovd_fwd_abort_actions(struct ovd_fwd_item *item)
{
	int side;

	for (side = 0; side < 2; side++) {
		struct ovd_fwd_half *half = &item->half[side];

		if (!half->sock)
			continue;
		if (half->kind == OVD_KIND_TCP)
			tcp_abort(half->sk, ECONNABORTED);
		else if (half->kind == OVD_KIND_VSOCK)
			kernel_sock_shutdown(half->sock, SHUT_RDWR);
	}
}

static void ovd_fwd_finish_pair(struct ovd_fwd_item *item)
{
	struct ovd_fwd_ctx *ctx = item->ctx;
	unsigned long flags;
	bool release;
	bool aborted;
	bool queued;
	int side;

	spin_lock_irqsave(&ctx->lock, flags);
	if (item->dead) {
		spin_unlock_irqrestore(&ctx->lock, flags);
		return;
	}
	item->dead = true;
	release = item->releasing || ctx->releasing;
	if (release)
		item->releasing = true;
	aborted = item->end_kind != END_COMPLETED;
	ovd_fwd_cleanup_begin_locked(item);
	spin_unlock_irqrestore(&ctx->lock, flags);

	for (side = 0; side < 2; side++)
		ovd_fwd_unhook_half(&item->half[side]);
	if (aborted)
		ovd_fwd_abort_actions(item);
	queued = ovd_fwd_pair_record(item);
	ovd_fwd_schedule_free(item);
	if (!queued)
		ovd_fwd_item_put(item);
}

static void ovd_fwd_finish_watch(struct ovd_fwd_item *item)
{
	struct ovd_fwd_ctx *ctx = item->ctx;
	struct ovd_fwd_half *half = &item->half[0];
	unsigned long flags;
	bool release;
	bool drop_base = false;

	spin_lock_irqsave(&ctx->lock, flags);
	if (item->dead) {
		spin_unlock_irqrestore(&ctx->lock, flags);
		return;
	}
	item->dead = true;
	release = item->releasing || ctx->releasing;
	if (release) {
		item->releasing = true;
		ovd_fwd_cleanup_begin_locked(item);
		drop_base = !item->record_queued;
		if (!list_empty(&item->live_node)) {
			list_del_init(&item->live_node);
			ctx->live_watches--;
		}
	}
	spin_unlock_irqrestore(&ctx->lock, flags);

	ovd_fwd_unhook_half(half);
	if (release && half->sock)
		kernel_sock_shutdown(half->sock, SHUT_RDWR);
	ovd_fwd_schedule_free(item);
	if (drop_base)
		ovd_fwd_item_put(item);
}

static void ovd_fwd_free_work(struct work_struct *work)
{
	struct ovd_fwd_item *item = container_of(work, struct ovd_fwd_item,
							 free_work.work);
	struct ovd_fwd_ctx *ctx = item->ctx;
	unsigned int module_puts = 0;
	int abandoned_work_refs;
	int side;

	/* Rule 3: wait for any pair run queued before its id left the table. */
	cancel_work_sync(&item->work);
	abandoned_work_refs = atomic_xchg(&item->work_refs, 0);
	while (abandoned_work_refs-- > 0)
		ovd_fwd_item_put(item);
	for (side = 0; side < (item->watch ? 1 : 2); side++) {
		struct ovd_fwd_half *half = &item->half[side];

		if (half->sock) {
			sockfd_put(half->sock);
			half->sock = NULL;
		}
		if (half->module_ref && !half->keep_module)
			module_puts++;
		if (half->keep_module)
			pr_warn_ratelimited("ovd_fwd: retaining module reference for TCP socket with ULP\n");
	}
	atomic_dec(&ctx->cleanup_pending);
	wake_up_all(&ctx->wait);
	ovd_fwd_item_put(item);
	while (module_puts--)
		module_put(THIS_MODULE);
}

static void ovd_fwd_work(struct work_struct *work)
{
	struct ovd_fwd_item *item = container_of(work, struct ovd_fwd_item, work);
	unsigned long again = 0;
	unsigned long pending;
	void *scratch = NULL;
	int budget[2] = { FWD_RUN_BUDGET, FWD_RUN_BUDGET };
	int direction;

	if (item->watch) {
		ovd_fwd_finish_watch(item);
		goto done;
	}
	if (ovd_fwd_pair_aborted(item)) {
		ovd_fwd_finish_pair(item);
		goto done;
	}
	scratch = mempool_alloc(ovd_fwd_scratch_pool, GFP_KERNEL);
	if (!scratch) {
		ovd_fwd_request_abort(item, CAUSE_SOCKET, 0, ENOMEM);
		ovd_fwd_finish_pair(item);
		goto done;
	}
	while ((pending = atomic_long_xchg(&item->pending, 0)) != 0) {
		if (ovd_fwd_pair_aborted(item)) {
			ovd_fwd_finish_pair(item);
			goto out;
		}
		for (direction = 0; direction < 2; direction++) {
			unsigned long bit = direction ? EV_DIR_B : EV_DIR_A;
			int result;

			if (!(pending & bit))
				continue;
			if (budget[direction] <= 0) {
				again |= bit;
				continue;
			}
			if (item->mode == OVD_FWD_MODE_STREAM)
				result = ovd_fwd_stream_step(item, direction, scratch,
							     &budget[direction]);
			else
				result = ovd_fwd_datagram_step(item, direction, scratch,
							       &budget[direction]);
			if (result < 0) {
				ovd_fwd_request_abort(item, CAUSE_SOCKET,
						      item->err_side, -result);
				break;
			}
			if (result == OVD_STEP_MORE)
				again |= bit;
		}
		if (ovd_fwd_pair_aborted(item)) {
			ovd_fwd_finish_pair(item);
			goto out;
		}
		if (ovd_fwd_pair_completed(item)) {
			ovd_fwd_finish_pair(item);
			goto out;
		}
		if (item->mode == OVD_FWD_MODE_STREAM &&
		    item->dir[0].source_end == SOURCE_CLEAN &&
		    item->dir[1].source_end == SOURCE_CLEAN &&
		    item->dir[0].target_shut && item->dir[1].target_shut) {
			ovd_fwd_request_complete(item);
			ovd_fwd_finish_pair(item);
			goto out;
		}
		if (again)
			break;
	}
	if (again)
		ovd_fwd_kick(item, again);
out:
	mempool_free(scratch, ovd_fwd_scratch_pool);
done:
	atomic_dec(&item->work_refs);
	ovd_fwd_item_put(item);
}

static struct ovd_fwd_item *ovd_fwd_find_live_locked(struct ovd_fwd_ctx *ctx,
						     u64 id)
{
	struct ovd_fwd_item *item;

	list_for_each_entry(item, &ctx->live, live_node) {
		if (item->id == id)
			return item;
	}
	return NULL;
}

static bool ovd_fwd_id_is_live(struct ovd_fwd_ctx *ctx, u64 id)
{
	unsigned long flags;
	bool live;

	spin_lock_irqsave(&ctx->lock, flags);
	live = !!ovd_fwd_find_live_locked(ctx, id);
	spin_unlock_irqrestore(&ctx->lock, flags);
	return live;
}

static long ovd_fwd_abort_id(struct ovd_fwd_ctx *ctx, u64 id)
{
	struct ovd_fwd_item *item;
	unsigned long flags;
	bool found;

	mutex_lock(&ctx->ops_lock);
	spin_lock_irqsave(&ctx->lock, flags);
	item = ovd_fwd_find_live_locked(ctx, id);
	found = !!item;
	if (item) {
		if (item->watch) {
			if (item->end_kind == END_NONE)
				ovd_fwd_record_watch_locked(item, END_WATCH_ABORTED,
							    CAUSE_DETACHED, 0, 0);
			else
				ovd_fwd_kick(item, EV_FINISH);
		} else {
			if (item->end_kind == END_NONE) {
				item->end_kind = END_ABORTED;
				item->end_cause = CAUSE_DETACHED;
			}
			ovd_fwd_kick(item, EV_FINISH);
		}
	}
	spin_unlock_irqrestore(&ctx->lock, flags);
	if (!found) {
		mutex_unlock(&ctx->ops_lock);
		return -ENOENT;
	}
	wait_event(ctx->wait,
		   !ovd_fwd_id_is_live(ctx, id) &&
		   atomic_read(&ctx->cleanup_pending) == 0);
	mutex_unlock(&ctx->ops_lock);
	return 0;
}

static long ovd_fwd_inventory(struct ovd_fwd_ctx *ctx,
			      struct ovd_fwd_inventory __user *user)
{
	struct ovd_fwd_inventory inventory = {};
	unsigned long flags;

	spin_lock_irqsave(&ctx->lock, flags);
	inventory.live_pairs = ctx->live_pairs;
	inventory.live_watches = ctx->live_watches;
	inventory.queued_ends = ctx->queued_ends;
	spin_unlock_irqrestore(&ctx->lock, flags);
	inventory.attached = atomic64_read(&ctx->attached);
	inventory.completed = atomic64_read(&ctx->completed);
	inventory.aborted_peer = atomic64_read(&ctx->aborted_peer);
	inventory.aborted_socket = atomic64_read(&ctx->aborted_socket);
	inventory.aborted_frame = atomic64_read(&ctx->aborted_frame);
	inventory.aborted_detached = atomic64_read(&ctx->aborted_detached);
	inventory.unclassified_eof = atomic64_read(&ctx->unclassified_eof);
	inventory.dgram_oversize = atomic64_read(&ctx->dgram_oversize);
	return copy_to_user(user, &inventory, sizeof(inventory)) ? -EFAULT : 0;
}

static long ovd_fwd_pair_state(struct ovd_fwd_ctx *ctx,
			       struct ovd_fwd_pair_state __user *user)
{
	struct ovd_fwd_pair_state state;
	struct ovd_fwd_item *item;
	unsigned long flags;
	int side;

	if (copy_from_user(&state, user, sizeof(state)))
		return -EFAULT;
	spin_lock_irqsave(&ctx->lock, flags);
	item = ovd_fwd_find_live_locked(ctx, state.id);
	if (!item || item->watch) {
		spin_unlock_irqrestore(&ctx->lock, flags);
		return -ENOENT;
	}
	for (side = 0; side < 2; side++) {
		state.source_end[side] = item->dir[side].source_end;
		state.target_shut[side] = item->dir[side].target_shut;
		state.moved[side] = item->dir[side].moved;
	}
	spin_unlock_irqrestore(&ctx->lock, flags);
	memset(state.reserved, 0, sizeof(state.reserved));
	return copy_to_user(user, &state, sizeof(state)) ? -EFAULT : 0;
}
