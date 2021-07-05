import subprocess
import conftest
import os
import socket
from tempfile import TemporaryDirectory


DEBUG_STAGE2 = os.getenv("TEST_DEBUG_STAGE2", False)


def test_stage2(helpers: conftest.Helpers) -> None:
    root = helpers.root()
    proc = subprocess.run(["cargo", "build"], cwd=root.joinpath("..", "src", "stage2"))
    assert proc.returncode == 0
    with TemporaryDirectory() as temp, helpers.busybox_image() as image:
        sock_path = f"{temp}/sock"
        extra_args = [
            "-drive",
            f"index=1,id=drive2,file={image},format=raw,if=none",
            "-device",
            "virtio-blk-pci,drive=drive2,serial=vmsh0,bootindex=2,serial=vmsh0",
            "-device",
            "virtio-serial",
            "-chardev",
            f"socket,path={sock_path},server,nowait,id=char0",
            "-device",
            "virtconsole,chardev=char0,id=vmsh",
        ]

        with helpers.spawn_qemu(helpers.notos_image(), extra_args) as vm:
            client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            client.connect(sock_path)

            vm.wait_for_ssh()
            cmd = ["strace", "-f"] if DEBUG_STAGE2 else []
            cmd += [
                "/vmsh/src/stage2/target/debug/stage2",
                "/bin/sh",
                "-c",
                "echo works",
            ]

            res = vm.ssh_cmd(cmd, check=False, stdout=None)
            assert res.returncode == 0
            out = client.recv(4096).decode("utf-8")
            print(out)
            lines = out.strip().split("\r\n")
            assert lines == ["works", "process finished with exit status: 0"]
