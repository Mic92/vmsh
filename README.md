# vmsh
Spawn debug container with shell access in virtual machines

# Usage

- Run `just pts` in one terminal to get a `/dev/pts/x`.
- Run `just qemu` in another terminal to spawn a VM.
- Run `just attach-qemu-sh /dev/pts/x` in another terminal to attach the first terminal to the shell which is spawned into the VM.


# Related work

Build small vm images with [microvm.nix](https://github.com/astro/microvm.nix)
