# vmsh

Lightweight virtual machines (VMs) are prominently adopted for improved
performance and dependability in cloud envi- ronments. To reduce boot up times
and resource utilisation, they are usually “pre-baked" with only the minimal
kernel and userland strictly required to run an application. This in- troduces a
fundamental trade-off between the advantages of lightweight VMs and available
services within a VM, usually leaning towards the former.  We propose VMSH, a
hypervisor-agnostic abstraction that enables on-demand attachment of services to
a running VM— allowing developers to provide minimal, lightweight images without
compromising their functionality. The additional applications are made available
to the guest via a file system image. To ensure that the newly added services do
not affect the original applications in the VM, VMSH uses lightweight isolation
mechanisms based on containers.  We evaluate VMSH on multiple KVM-based
hypervisors and Linux LTS kernels and show that: (i) VMSH adds no overhead for
the applications running in the VM, (ii) de-bloating im- ages from the Docker
registry can save up to 60% of their size on average, and (iii) VMSH enables
cloud providers to offer services to customers, such as recovery shells, without
interfering with their VM’s execution.

# Reproducing the paper results

VMSH was published in Eurosys 2022. To reproduce the results shown in the
evaluation of the paper, we provide [dedicated documentation](EVALUATION.md).

# Usage

- Run `just pts` in one terminal to get a `/dev/pts/x`.
- Run `just qemu` in another terminal to spawn a VM.
- Run `just attach-qemu-sh /dev/pts/x` in another terminal to attach the first terminal to the shell which is spawned into the VM.


# Related work

Build small vm images with [microvm.nix](https://github.com/astro/microvm.nix)
