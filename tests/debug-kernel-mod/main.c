#include <linux/kernel.h> /* Needed for KERN_INFO */
#include <linux/module.h> /* Needed by all modules */

#include <asm/io.h>             /* Needed for dump_processes */
#include <linux/sched/signal.h> /* Needed for dump_processes */

void dump_processes(void) {
  struct task_struct *g;
  void *cr3 = (void*)read_cr3_pa();

  rcu_read_lock();
  for_each_process(g) {
    if (g->mm) {
      printk("%s --> 0x%lx\n", g->comm, (uintptr_t)virt_to_phys(g->mm->pgd));
    } else {
      printk("%s -> 0x%lx\n", g->comm, (uintptr_t)cr3);
    }
  }
  rcu_read_unlock();
}

int init_module(void) {
  printk(KERN_INFO "load module...\n");
  // Just an example to debug something in the kernel
  dump_processes();
  return 0;
}

void cleanup_module(void) {}

MODULE_AUTHOR("joerg@thalheim.io");
MODULE_DESCRIPTION("random kernel hacks");
MODULE_LICENSE("GPL");
