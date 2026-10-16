/* SPDX-License-Identifier: GPL-2.0 WITH Linux-syscall-note */
/* evmod: spike-only control interface. Not a design contract. */
#ifndef EVMOD_UAPI_H
#define EVMOD_UAPI_H
#include <linux/types.h>
#include <linux/ioctl.h>

struct evmod_attach {
	__s32 fd_a;   /* first socket (AF_VSOCK stream or AF_INET TCP), already connected */
	__s32 fd_b;   /* second socket, same rule */
	__u32 flags;  /* reserved, must be 0 */
	__u32 id;     /* out: pair id */
};

/* Both sockets stay owned by the caller's file table as well; the module takes its own file reference. */
#define EVMOD_ATTACH _IOWR('E', 1, struct evmod_attach)
/* Synchronous: returns once callbacks are restored and the module's file references are dropped. */
#define EVMOD_DETACH _IOW('E', 2, __u32)
#endif
