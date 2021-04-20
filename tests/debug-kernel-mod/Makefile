obj-m += debug-kernel-mod.o

debug-kernel-mod-objs := main.o

#KERNELDIR ?= /lib/modules/$(shell uname -r)/build
KERNELDIR ?= ../../../linux

all:
	make -C $(KERNELDIR) M=$(PWD) modules

clean:
	make -C $(KERNELDIR) M=$(PWD) clean
