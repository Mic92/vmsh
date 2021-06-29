import subprocess
import conftest
import socket
import os
from threading import Thread
from typing import List


class VsockServer:
    def __init__(self, port: int) -> None:
        self.port = port
        self.s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
        self.s.bind((socket.VMADDR_CID_HOST, port))
        self.s.listen()
        self.connections: List[socket.socket] = []
        Thread(target=self.accept).start()

    def accept(self) -> None:
        (conn, (remote_cid, remote_port)) = self.s.accept()
        print(
            f"Received connection from {remote_cid}:{remote_port} on port {self.port}"
        )
        self.connections.append(conn)


DEBUG_STAGE2 = os.getenv("TEST_DEBUG_STAGE2", False)


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

        monitor_server = VsockServer(9998)
        pty_server = VsockServer(9999)

        with helpers.spawn_qemu(helpers.notos_image(), extra_args) as vm:
            vm.wait_for_ssh()
            cmd = ["strace", "-f"] if DEBUG_STAGE2 else []
            cmd.append("/vmsh/src/stage2/target/debug/stage2")

            res = vm.ssh_cmd(cmd, check=False, stdout=None)
            assert len(monitor_server.connections) == 1
            print(monitor_server.connections[0].recv(4096).decode("utf-8"))
            assert len(pty_server.connections) == 1
            assert res.returncode == 0
