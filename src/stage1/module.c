#include <linux/module.h>

#include "stage1.h"

#define MAX_STAGE2_ARGS 254
static int stage2_argc;
static char *stage2_argv[MAX_STAGE2_ARGS];

// FIXME: Right now this is a kernel module in future, this should be replaced
// something to be injectable into VMs.
int init_module(void) {
  return init_vmsh_stage1(stage2_argc, stage2_argv);
}

void cleanup_module(void) {
  cleanup_vmsh_stage1();
}

module_param_array(stage2_argv, charp, &stage2_argc, 0);

MODULE_AUTHOR("joerg@thalheim.io");
MODULE_DESCRIPTION("Mount block device and launch intial vmsh process");
MODULE_LICENSE("GPL");
