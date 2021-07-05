#include <linux/kernel.h> /* Needed for KERN_INFO */
#include <linux/module.h> /* Needed by all modules */
#include <linux/delay.h> /* usleep_range */

#include <asm/io.h>             /* Needed for dump_processes */
#include <linux/sched/signal.h> /* Needed for dump_processes */

struct kernel_symbol* start_ksymtab(void);
struct kernel_symbol* stop_ksymtab(void);
struct kernel_symbol* start_ksymtab_gpl(void);
struct kernel_symbol* stop_ksymtab_gpl(void);
struct kernel_symbol* start_ksymtab_start(void);
struct kernel_symbol* stop_ksymtab_stop(void);

const char* start_ksymtab_strings(void);
const char* stop_ksymtab_strings(void);

void dump_processes(void) {
  size_t count = 0;
  const struct kernel_symbol *sym;
  void *cr3 = (void*)read_cr3_pa();

  printk("cr3=0x%lx\n", (uintptr_t)cr3);

  uintptr_t ksymtab_start = (uintptr_t)start_ksymtab_strings();


  for (sym = start_ksymtab(); sym < stop_ksymtab(); sym++) {
    void* ptr = (char*)offset_to_ptr(&sym->name_offset);
    int contained = (u64)start_ksymtab_strings() <= (u64)ptr && (u64)ptr < (u64)stop_ksymtab_strings();
    printk("=== %s %x 0x%lx %d %lx\n", (char*)ptr, sym->name_offset, (unsigned long)ptr, contained, ksymtab_start - (uintptr_t)sym);
    usleep_range(100, 101);
    count += 1;
  }
  for (sym = start_ksymtab_gpl(); sym < stop_ksymtab_gpl(); sym++) {
    void* ptr = (char*)offset_to_ptr(&sym->name_offset);
    int contained = (u64)start_ksymtab_strings() <= (u64)ptr && (u64)ptr < (u64)stop_ksymtab_strings();
    printk("=== %s %x 0x%lx %d %lx\n", (char*)ptr, sym->name_offset, (unsigned long)ptr, contained, ksymtab_start - (uintptr_t)sym);
    usleep_range(100, 101);
    count += 1;
  }
  printk("################# count=%zu ksymtab_start=0x%lx ################\n", count, ksymtab_start);


  // struct task_struct *g;
  //rcu_read_lock();
  //for_each_process(g) {
  //  if (g->mm) {
  //    printk("%s --> 0x%lx\n", g->comm, (uintptr_t)virt_to_phys(g->mm->pgd));
  //  } else {
  //    printk("%s -> 0x%lx\n", g->comm, (uintptr_t)cr3);
  //  }
  //}
  //rcu_read_unlock();
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
