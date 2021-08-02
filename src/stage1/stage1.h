#pragma once

#define MAX_STAGE2_ARGS 256
#define RESERVED_STAGE2_ARGS 2
#define MAX_DEVICES 3

struct Stage1Args {
    unsigned long long device_addrs[MAX_DEVICES];
    char* argv[MAX_STAGE2_ARGS];
};

extern struct Stage1Args VMSH_STAGE1_ARGS;

int init_vmsh_stage1(void);
void cleanup_vmsh_stage1(void);
