use kvm_ioctls::{IoEventAddress, VmFd};

fn ioctl(s: &str) {
    println!("{}", s);
}

pub fn attach_blk_dev() {
    ioctl("KVM_CREATE_DEVICE");
    ioctl("KVM_IRQFD");
    ioctl("KVM_IOEVENTFD");
    let foo = IoEventAddress::Pio(2);
}

#[cfg(test)]
mod test {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}

