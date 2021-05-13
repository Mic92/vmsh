#include <linux/module.h>

#include "stage1.h"

// FIXME: Right now this is a kernel module in future, this should be replaced
// something to be injectable into VMs.
int init_module(void) {
  return init_vmsh_stage1();
}

void cleanup_module(void) {
  cleanup_vmsh_stage1();
}

MODULE_AUTHOR("joerg@thalheim.io");
MODULE_DESCRIPTION("Mount block device and launch intial vmsh process");
MODULE_LICENSE("GPL");
