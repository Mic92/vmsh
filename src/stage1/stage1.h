#pragma once

int init_vmsh_stage1(int devices_num, unsigned long long* devices, int stage2_argc, char** stage2_argv);
void cleanup_vmsh_stage1(void);
