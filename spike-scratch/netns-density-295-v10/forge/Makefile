# Out-of-tree build of the V-10 forge module against the installed kernel
# headers (the metal host has a headers-only package at
# /lib/modules/$(uname -r)/build, no full source tree, so this is the only
# way to produce the .ko). vermagic matches the 7.0.0-29-generic guest kernel.
obj-m += vsock_forge.o

KDIR ?= /lib/modules/$(shell uname -r)/build

all:
	$(MAKE) -C $(KDIR) M=$(CURDIR) modules

clean:
	$(MAKE) -C $(KDIR) M=$(CURDIR) clean
