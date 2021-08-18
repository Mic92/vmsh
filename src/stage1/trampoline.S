.intel_syntax noprefix

.global _init_vmsh
.type _init_vmsh,function
.text

_init_vmsh:
  // save all general purpose register
  pushf
  push rax
  push rbx
  push rcx
  push rdx
  push rbp
  push rsi
  push rdi
  push r8
  push r9
  push r10
  push r11
  push r12
  push r13
  push r14
  push r15

  // call rust code
  call [init_vmsh@GOTPCREL + rip]

  // restore general purpose register
  pop r15
  pop r14
  pop r13
  pop r12
  pop r11
  pop r10
  pop r9
  pop r8
  pop rdi
  pop rsi
  pop rbp
  pop rdx
  pop rcx
  pop rbx
  pop rax
  popf

  // return to code we came from
  jmp [VMSH_STAGE1_PC@GOTPCREL + rip]
