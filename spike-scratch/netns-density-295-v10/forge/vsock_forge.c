// V-10 adversarial-guest apparatus (THROWAWAY SPIKE, netns-density-295).
//
// Forges the on-wire virtio-vsock `src_cid` of every packet this guest emits,
// at the packet level, by overriding the return value of the guest virtio
// transport's `virtio_transport_get_local_cid()`. That function is the single
// source the common layer reads when it stamps `hdr->src_cid` in
// `virtio_transport_alloc_skb()` (net/vmw_vsock/virtio_transport_common.c), so
// overriding its return makes the guest kernel build and submit virtqueue
// packets whose header `src_cid` is a value the host NEVER assigned this VM.
//
// This is the "patched guest driver / guest kernel module" apparatus sanctioned
// for V-10: it runs ONLY inside the throwaway guest, loaded from the initramfs
// after the stock vsock modules. It is never loaded on the host.
//
// forge_cid = 0  -> passthrough (no forge; control)
// forge_cid = N  -> every outgoing packet carries src_cid = N
//
// The handler counts hits and logs the first few overrides so the guest serial
// log independently proves the forge fired and what it forged.

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/kprobes.h>
#include <linux/ptrace.h>
#include <linux/atomic.h>

static unsigned int forge_cid;
module_param(forge_cid, uint, 0644);
MODULE_PARM_DESC(forge_cid,
	"nonzero => override virtio_transport_get_local_cid() return (forge src_cid)");

static atomic_t hits = ATOMIC_INIT(0);
static atomic_t forged = ATOMIC_INIT(0);

static int ret_handler(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	unsigned long real = regs_return_value(regs);

	atomic_inc(&hits);
	if (forge_cid) {
		regs_set_return_value(regs, forge_cid);
		if (atomic_inc_return(&forged) <= 8)
			pr_info("vsock_forge: override get_local_cid real=%lu -> forged=%u\n",
				real, forge_cid);
	}
	return 0;
}

static struct kretprobe krp = {
	.handler = ret_handler,
	.maxactive = 64,
	.kp.symbol_name = "virtio_transport_get_local_cid",
};

static int __init forge_init(void)
{
	int r = register_kretprobe(&krp);

	if (r < 0) {
		pr_err("vsock_forge: register_kretprobe(%s) failed %d\n",
		       krp.kp.symbol_name, r);
		return r;
	}
	pr_info("vsock_forge: armed on %s forge_cid=%u addr=%p\n",
		krp.kp.symbol_name, forge_cid, krp.kp.addr);
	return 0;
}

static void __exit forge_exit(void)
{
	unregister_kretprobe(&krp);
	pr_info("vsock_forge: unregister hits=%d forged=%d missed=%d\n",
		atomic_read(&hits), atomic_read(&forged), krp.nmissed);
}

module_init(forge_init);
module_exit(forge_exit);
MODULE_LICENSE("GPL");
MODULE_AUTHOR("netns-density-295 V-10 spike");
MODULE_DESCRIPTION("V-10: forge guest vsock src_cid via kretprobe on virtio_transport_get_local_cid");
