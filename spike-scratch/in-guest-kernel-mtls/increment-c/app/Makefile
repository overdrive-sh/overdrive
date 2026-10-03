# Spike A3: build with Unikraft's make/Kconfig system directly (kraft not used).
#   make UK_ROOT=<unikraft> UK_LWIP=<lib-lwip> UK_BUILD=<out dir> [target]
UK_ROOT  ?= $(PWD)/../unikraft
UK_LWIP  ?= $(PWD)/../lib-lwip
UK_BUILD ?= $(PWD)/build

all:
	@$(MAKE) -C $(UK_ROOT) A=$(PWD) L=$(UK_LWIP) O=$(UK_BUILD)

$(MAKECMDGOALS):
	@$(MAKE) -C $(UK_ROOT) A=$(PWD) L=$(UK_LWIP) O=$(UK_BUILD) $(MAKECMDGOALS)
