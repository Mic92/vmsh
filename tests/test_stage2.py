import subprocess
import conftest


def test_stage2(helpers: conftest.Helpers) -> None:
    root = helpers.root()
    proc = subprocess.run(["cargo", "build"], cwd=root.joinpath("..", "src", "stage2"))
    assert proc.returncode == 0
    # TODO
    with helpers.busybox_image() as image:
        extra_args = [
            "-drive",
            f"index=1,id=drive2,file={image},format=raw,if=none",
            "-device",
            "virtio-blk-pci,drive=drive2,serial=vmsh0,bootindex=2,serial=vmsh0",
        ]

        with helpers.spawn_qemu(helpers.notos_image(), extra_args) as vm:
            vm.wait_for_ssh()
            # vm.attach()
            res = vm.ssh_cmd(
                ["/vmsh/src/stage2/target/debug/stage2"], check=False, stdout=None
            )
            assert res.returncode == 0
