// SPDX-License-Identifier: GPL-2.0
// Disposable stock socket-API glue; this is a kernel module, not eBPF.
#include <linux/module.h>
#include <linux/net.h>
#include <linux/in.h>
#include <linux/vm_sockets.h>
#include <linux/kthread.h>
#include <linux/delay.h>
#include <linux/slab.h>
#include <linux/proc_fs.h>
#include <linux/seq_file.h>
#include <linux/completion.h>
#include <net/sock.h>
#include <net/net_namespace.h>

#define MAX_SESS 16400
#define BOUND 60000
struct wire { u8 kind, reserved; __be16 port; __be32 addr, len; } __packed;
struct session { struct socket *vs, *inet; struct task_struct *task, *rx; int mode; atomic_t stop; struct completion rx_done; };
static struct session sessions[MAX_SESS];
static struct socket *listener;
static struct task_struct *accept_task;
static bool stopping;
static atomic64_t accepted, closed, to_net, to_peer, zero_udp, truncated, rejected, send_waits;

static bool done(struct session *s) { return READ_ONCE(stopping) || atomic_read(&s->stop) || kthread_should_stop(); }
static int send_all(struct session *s, struct socket *sock, const void *buf, int n) {
 unsigned long limit = jiffies + 15 * HZ; int off = 0;
 while (off < n && !done(s)) {
  struct msghdr msg = { .msg_flags = MSG_DONTWAIT | MSG_NOSIGNAL };
  struct kvec vec = { .iov_base = (void *)buf + off, .iov_len = n - off };
  int rc = kernel_sendmsg(sock, &msg, &vec, 1, n - off);
  if (rc > 0) { off += rc; limit = jiffies + 15 * HZ; }
  else if (rc == -EAGAIN) { atomic64_inc(&send_waits); if (time_after(jiffies, limit)) return -ETIMEDOUT; msleep(1); }
  else return rc ? rc : -EPIPE;
 }
 return off == n ? 0 : -ECANCELED;
}
static int recv_all(struct session *s, void *buf, int n) {
 unsigned long limit = jiffies + 3600 * HZ; int off = 0;
 while (off < n && !done(s)) {
  struct msghdr msg = {}; struct kvec vec = { .iov_base = buf + off, .iov_len = n - off };
  int rc = kernel_recvmsg(s->vs, &msg, &vec, 1, n - off, 0);
  if (rc > 0) { off += rc; limit = jiffies + 3600 * HZ; }
  else if (rc == -EAGAIN) { if (time_after(jiffies, limit)) return -ETIMEDOUT; msleep(1); }
  else return rc ? rc : -EPIPE;
 }
 return off == n ? 0 : -ECANCELED;
}
static int emit(struct session *s, u8 kind, struct sockaddr_in *addr, void *data, int n) {
 struct wire h = { .kind = kind, .port = addr ? addr->sin_port : 0, .addr = addr ? addr->sin_addr.s_addr : 0, .len = cpu_to_be32(n) };
 int rc = send_all(s, s->vs, &h, sizeof(h));
 return rc ? rc : send_all(s, s->vs, data, n);
}
static int inet_rx(void *arg) {
 struct session *s = arg; void *buf = kmalloc(BOUND, GFP_KERNEL); int rc = 0;
 if (!buf) return -ENOMEM;
 while (!done(s)) {
  struct sockaddr_in addr = {}; struct msghdr msg = { .msg_name = &addr, .msg_namelen = sizeof(addr) };
  struct kvec vec = { .iov_base = buf, .iov_len = BOUND };
  int n = kernel_recvmsg(s->inet, &msg, &vec, 1, BOUND, s->mode == 1 ? 0 : MSG_TRUNC);
  if (done(s)) break;
  if (n == -EAGAIN) { msleep(1); continue; }
  if (n < 0) { rc = n; break; }
  if (s->mode == 1 && n == 0) { emit(s, 11, NULL, NULL, 0); break; }
  if (n > BOUND) { atomic64_inc(&truncated); emit(s, 30, &addr, NULL, 0); continue; }
  if (!n && s->mode != 1) atomic64_inc(&zero_udp);
  rc = emit(s, 10, s->mode == 1 ? NULL : &addr, buf, n);
  if (rc) break;
  atomic64_add(n, &to_peer);
 }
 kfree(buf); complete_all(&s->rx_done); return rc;
}
static int serve(void *arg) {
 struct session *s = arg; struct wire h; struct sockaddr_in addr = { .sin_family = AF_INET }; void *buf = kmalloc(BOUND, GFP_KERNEL); int rc;
 if (!buf) goto finish;
 rc = recv_all(s, &h, sizeof(h)); if (rc) goto free;
 s->mode = h.kind;
 if (be32_to_cpu(h.len) || s->mode < 1 || s->mode > 3) { atomic64_inc(&rejected); goto free; }
 rc = sock_create_kern(&init_net, AF_INET, s->mode == 1 ? SOCK_STREAM : SOCK_DGRAM, s->mode == 1 ? IPPROTO_TCP : IPPROTO_UDP, &s->inet);
 if (rc) goto free;
 if (s->mode != 1) s->inet->sk->sk_no_check_tx = 1; // Match Aya codec-required IPv4 UDP checksum policy in this owned benchmark.
 s->inet->sk->sk_rcvtimeo = msecs_to_jiffies(1000);
 addr.sin_port = h.port; addr.sin_addr.s_addr = h.addr;
 if (s->mode == 1) rc = kernel_connect(s->inet, (struct sockaddr_unsized *)&addr, sizeof(addr), 0);
 else { if (s->mode == 2) addr.sin_port = 0; rc = kernel_bind(s->inet, (struct sockaddr_unsized *)&addr, sizeof(addr)); }
 if (rc) { pr_info("shmproxy: setup mode=%d errno=%d\n", s->mode, rc); atomic64_inc(&rejected); emit(s, 30, NULL, NULL, 0); goto free; }
 kernel_getsockname(s->inet, (struct sockaddr *)&addr);
 if (emit(s, 20, &addr, NULL, 0)) goto free;
 init_completion(&s->rx_done);
 s->rx = kthread_create(inet_rx, s, "shmprx-%ld", s - sessions);
 if (IS_ERR(s->rx)) { s->rx = NULL; goto free; }
 get_task_struct(s->rx); wake_up_process(s->rx);
 while (!done(s)) {
  int n; struct msghdr msg; struct kvec vec;
  rc = recv_all(s, &h, sizeof(h)); if (rc) break;
  n = be32_to_cpu(h.len); if (n > BOUND || h.kind != 10) {
   if (h.kind == 11 && n == 0 && s->mode == 1) { kernel_sock_shutdown(s->inet, SHUT_WR); break; }
   if (h.kind != 12) atomic64_inc(&rejected); break;
  }
  rc = recv_all(s, buf, n); if (rc) break;
  if (s->mode == 1) rc = send_all(s, s->inet, buf, n);
  else {
   addr.sin_port = h.port; addr.sin_addr.s_addr = h.addr;
   msg = (struct msghdr){ .msg_name = &addr, .msg_namelen = sizeof(addr), .msg_flags = MSG_DONTWAIT | MSG_NOSIGNAL };
   vec = (struct kvec){ .iov_base = buf, .iov_len = n };
   rc = kernel_sendmsg(s->inet, &msg, &vec, 1, n); if (rc == n) rc = 0;
  }
  if (rc) break; atomic64_add(n, &to_net);
 }
 // A TCP half-close drains the independent reverse worker before socket release.
 if (h.kind == 11 && s->mode == 1) { unsigned long limit = jiffies + 15 * HZ; while (!done(s) && !completion_done(&s->rx_done) && time_before(jiffies, limit)) msleep(1); }
free:
 atomic_set(&s->stop, 1);
 if (s->inet) kernel_sock_shutdown(s->inet, SHUT_RDWR);
 if (s->rx) { kthread_stop(s->rx); put_task_struct(s->rx); s->rx = NULL; }
 if (s->inet) { sock_release(s->inet); s->inet = NULL; }
 kfree(buf);
finish:
 kernel_sock_shutdown(s->vs, SHUT_RDWR); sock_release(s->vs); s->vs = NULL;
 atomic64_inc(&closed); return 0;
}
static int accept_loop(void *arg) {
 int slot = 0;
 while (!READ_ONCE(stopping) && !kthread_should_stop()) {
  struct socket *sock = NULL; int rc = kernel_accept(listener, &sock, O_NONBLOCK);
  if (rc == -EAGAIN) { msleep(1); continue; } if (rc) break;
  if (slot == MAX_SESS) { sock_release(sock); atomic64_inc(&rejected); continue; }
  sessions[slot].vs = sock; sock->sk->sk_rcvtimeo = msecs_to_jiffies(1000); atomic64_inc(&accepted);
  sessions[slot].task = kthread_create(serve, &sessions[slot], "shmps-%d", slot);
  if (!IS_ERR(sessions[slot].task)) { get_task_struct(sessions[slot].task); wake_up_process(sessions[slot].task); } else { sock_release(sock); sessions[slot].vs = NULL; }
  slot++;
 }
 return 0;
}
static int stats(struct seq_file *m, void *v) {
 seq_printf(m, "accepted=%lld closed=%lld to_net=%lld to_peer=%lld zero_udp=%lld truncated=%lld rejected=%lld send_waits=%lld\n", atomic64_read(&accepted), atomic64_read(&closed), atomic64_read(&to_net), atomic64_read(&to_peer), atomic64_read(&zero_udp), atomic64_read(&truncated), atomic64_read(&rejected), atomic64_read(&send_waits)); return 0;
}
static int __init proxy_init(void) {
 struct sockaddr_vm addr = { .svm_family = AF_VSOCK, .svm_port = 29500, .svm_cid = VMADDR_CID_HOST }; int rc;
 rc = sock_create_kern(&init_net, AF_VSOCK, SOCK_STREAM, 0, &listener); if (rc) return rc;
 rc = kernel_bind(listener, (struct sockaddr_unsized *)&addr, sizeof(addr)); if (rc) goto fail;
 rc = kernel_listen(listener, MAX_SESS); if (rc) goto fail;
 accept_task = kthread_run(accept_loop, NULL, "shmp-accept"); if (IS_ERR(accept_task)) { rc = PTR_ERR(accept_task); goto fail; }
 proc_create_single("shmproxy", 0444, NULL, stats); pr_info("shmproxy: kernel AF_VSOCK listener CID2:29500 loaded\n"); return 0;
fail: sock_release(listener); return rc;
}
static void __exit proxy_exit(void) {
 int i; remove_proc_entry("shmproxy", NULL); WRITE_ONCE(stopping, true);
 kthread_stop(accept_task); kernel_sock_shutdown(listener, SHUT_RDWR); sock_release(listener);
 for (i = 0; i < MAX_SESS; i++) if (sessions[i].task && !IS_ERR(sessions[i].task)) { kthread_stop(sessions[i].task); put_task_struct(sessions[i].task); }
 pr_info("shmproxy: drained accepted=%lld closed=%lld\n", atomic64_read(&accepted), atomic64_read(&closed));
}
module_init(proxy_init); module_exit(proxy_exit);
MODULE_LICENSE("GPL"); MODULE_DESCRIPTION("Bounded scratch host in-kernel TCP UDP vsock adapter");
