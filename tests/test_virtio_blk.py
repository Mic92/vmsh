import conftest
from multiprocessing import Process
import subprocess


def test_loading_virtio_mmio(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vm.wait_for_ssh()
        print("ssh available")
        res = vm.ssh_cmd(
            ["insmod", "/run/current-system/sw/lib/modules/virtio/virtio_mmio.ko"]
        )
        assert res.returncode == 0
        # assert that virtio_mmio is now loaded
        res = vm.ssh_cmd(["lsmod"])
        assert res.stdout
        assert res.stdout.find("virtio_mmio") >= 0


def test_virtio_device_space(helpers: conftest.Helpers) -> None:
    with helpers.spawn_qemu(helpers.notos_image()) as vm:
        vmsh = Process(target=helpers.run_vmsh_command, args=(["attach", str(vm.pid)],))
        vmsh.start()
        vm.wait_for_ssh()
        print("ssh available")

        mmio_config = "0x1000@0xd0000000:5"
        res = vm.ssh_cmd(
            [
                "insmod",
                "/run/current-system/sw/lib/modules/virtio/virtio_mmio.ko",
                f"device={mmio_config}",
            ]
        )
        print("stdout:\n", res.stdout)
        print("stderr:\n", res.stderr)

        res = vm.ssh_cmd(["dmesg"])
        print("stdout:\n", res.stdout)

        print("multiprocessing pid: ", vmsh.pid)

        # python sucks
        print(
            "kill children: ",
            # The following runs do not kill children (daemonization?) but lead
            # to tests appearing to terminate in a shell - but build CI will
            # never terminate because it sees not all processes have
            # terminated. See yourself how `ps aux | grep vmsh | wc -l`
            # measures increasing numbers of vmsh processes before and after
            # running test_virtio_blk.py when using on of the following:
            #
            # subprocess.run(["sudo", "pkill", "-P", f"{vmsh.pid}"], check=True),
            # subprocess.run(["sh", "-c", f"pkill -P {vmsh.pid}"], check=True),
            # (vmsh.kill() doesn't work either)
            #
            # The following magically removes all children though.
            subprocess.run(["sudo", "su", "-c", f"pkill -P {vmsh.pid}"], check=True),
        )
        print("kill: ", subprocess.run(["kill", f"{vmsh.pid}"]))
        vmsh.join(10)
        if vmsh.exitcode is None:
            print("termination needed ERROR")
            # vmsh.terminate()

        assert res.stdout.find("virtio_mmio: unknown parameter 'device' ignored") < 0
