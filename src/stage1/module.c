#include <linux/module.h>
#include <asm/io.h>
#include <linux/pgtable.h>

#include "stage1.h"

#define MAX_STAGE2_ARGS 254
static int stage2_argc;
static char *stage2_argv[MAX_STAGE2_ARGS];

#define MAX_DEVICES 254
static int devices_num;
static char *devices[3];
static char *phys_mem, *virt_mem;
static char* printk_addr;

// FIXME: Right now this is a kernel module in future, this should be replaced
// something to be injectable into VMs.
int init_module(void) {
  unsigned long long devs[3];
  size_t i;
  unsigned long mem = 0;
  void __iomem *baseptr;
  int (*printk_addr_func)(const char format, ...);

  for (i = 0; i < devices_num; i++) {
    if (kstrtoull(devices[i], 10, &devs[i])) {
      printk(KERN_ERR "stage1: invalid mmio address: %s\n", devices[i]);
      return -EINVAL;
    }
    printk(KERN_INFO "stage1: device address: 0x%llx\n", devs[i]);
  }

  if (phys_mem) {
    if (kstrtoul(phys_mem, 10, &mem)) {
      printk(KERN_ERR "stage1: invalid phys_mem address: %s\n", phys_mem);
      return -EINVAL;
    }
    printk("physical memory: 0x%lx -> 0x%lx", mem, mem + 0x2000);

    baseptr = ioremap(mem, 0x2000);
    if (!baseptr) {
      printk(KERN_ERR "stage1: cannot map phys_mem address: %lx\n", mem);
      return -ENOMEM;
    }
    memset(baseptr, 'A', 0x2000);
    iounmap(baseptr);
  }

  if (virt_mem) {
    if (kstrtoul(virt_mem, 10, (unsigned long*)&mem)) {
      printk(KERN_ERR "stage1: invalid virt_mem address: %s\n", virt_mem);
      return -EINVAL;
    }
    printk(KERN_INFO "stage1: virtual memory access: 0x%lx-0x%lx\n", mem, mem + 0x2000);

    memset((void*)mem, 'A', 0x2000);
  }

  if (printk_addr) {
    if (kstrtoul(printk_addr, 10, (unsigned long*)&printk_addr_func)) {
      return -EINVAL;
    }
    printk(KERN_ERR "stage1: printk: 0x%lx vs 0x%lx!\n", (unsigned long) printk, (unsigned long) printk_addr_func);
  }

  return init_vmsh_stage1(devices_num, devs, stage2_argc, stage2_argv);
}

void cleanup_module(void) {
  cleanup_vmsh_stage1();
}

// those parameter are used for testing
module_param(phys_mem, charp, 0);
module_param(virt_mem, charp, 0);
module_param(printk_addr, charp, 0);

module_param_array(devices, charp, &devices_num, 0);
module_param_array(stage2_argv, charp, &stage2_argc, 0);

MODULE_AUTHOR("joerg@thalheim.io");
MODULE_DESCRIPTION("Mount block device and launch intial vmsh process");
MODULE_LICENSE("GPL");
