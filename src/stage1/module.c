#include <linux/module.h>
#include <asm/io.h>
#include <linux/pgtable.h>

#include "stage1.h"

#define MAX_STAGE2_ARGS 256
#define RESERVED_STAGE2_ARGS 2

static char *phys_mem, *virt_mem;
static char* printk_addr;
static char* init_func;
static char* exit_func;
void (*cleanup_vmsh_stage1p)(void);

// FIXME: Right now this is a kernel module in future, this should be replaced
// something to be injectable into VMs.
int init_module(void) {
  unsigned long mem = 0;
  void __iomem *baseptr;
  int (*printk_addr_func)(const char format, ...);
  int (*init_vmsh_stage1p)(void);

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

  if (exit_func) {
    if (kstrtoul(exit_func, 10, (unsigned long*)&cleanup_vmsh_stage1p)) {
      printk(KERN_ERR "stage1: invalid exit_func: %s\n", virt_mem);
    }
  } else {
    printk(KERN_ERR "stage1: no exit_func passed\n");
    return -EINVAL;
  }

  if (init_func) {
    if (kstrtoul(init_func, 10, (unsigned long*)&init_vmsh_stage1p)) {
      printk(KERN_ERR "stage1: invalid init_func: %s\n", virt_mem);
      return -EINVAL;
    }
    return init_vmsh_stage1p();
  } else {
    printk(KERN_ERR "stage1: no init_func passed\n");
    return -EINVAL;
  }
}

void cleanup_module(void) {
  cleanup_vmsh_stage1p();
}

// those parameter are used for testing
module_param(phys_mem, charp, 0);
module_param(virt_mem, charp, 0);
module_param(printk_addr, charp, 0);
module_param(init_func, charp, 0);
module_param(exit_func, charp, 0);

MODULE_AUTHOR("joerg@thalheim.io");
MODULE_DESCRIPTION("Mount block device and launch intial vmsh process");
MODULE_LICENSE("GPL");
