// SPDX-License-Identifier: GPL-2.0

#include <linux/errno.h>
#include <linux/fs.h>
#include <linux/init.h>
#include <linux/miscdevice.h>
#include <linux/module.h>
#include <linux/uaccess.h>
#include <linux/workqueue.h>

#include <linux/ovd_fwd.h>

#ifndef OVD_FWD_MODULE_VERSION
#error "OVD_FWD_MODULE_VERSION must be supplied by cargo xtask kmod-build"
#endif

static struct workqueue_struct *ovd_fwd_workqueue;

static long ovd_fwd_ioctl(struct file *file, unsigned int command,
			  unsigned long argument)
{
	struct ovd_fwd_version version = {
		.abi_version = OVD_FWD_ABI_VERSION,
	};

	if (command != OVD_FWD_VERSION)
		return -ENOTTY;

	if (copy_to_user((void __user *)argument, &version, sizeof(version)))
		return -EFAULT;

	return 0;
}

static const struct file_operations ovd_fwd_fops = {
	.owner = THIS_MODULE,
	.unlocked_ioctl = ovd_fwd_ioctl,
};

static struct miscdevice ovd_fwd_device = {
	.minor = MISC_DYNAMIC_MINOR,
	.name = "ovd_fwd",
	.fops = &ovd_fwd_fops,
};

static int __init ovd_fwd_init(void)
{
	int error;

	ovd_fwd_workqueue = alloc_workqueue("ovd_fwd", WQ_PERCPU, 0);
	if (!ovd_fwd_workqueue)
		return -ENOMEM;

	error = misc_register(&ovd_fwd_device);
	if (error) {
		destroy_workqueue(ovd_fwd_workqueue);
		ovd_fwd_workqueue = NULL;
	}

	return error;
}

static void __exit ovd_fwd_exit(void)
{
	misc_deregister(&ovd_fwd_device);
	destroy_workqueue(ovd_fwd_workqueue);
}

module_init(ovd_fwd_init);
module_exit(ovd_fwd_exit);

MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("Overdrive event-driven flow forwarder");
MODULE_VERSION(OVD_FWD_MODULE_VERSION);
