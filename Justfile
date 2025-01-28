build-takeoff:
	cd src/takeoff && RUSTFLAGS="-C link-arg=-nostartfiles -C target-feature=+crt-static" cargo build --bin takeoff --release --target x86_64-unknown-linux-musl
	
	mkdir -p target/cpio

	cp target/x86_64-unknown-linux-musl/release/takeoff target/cpio/init
	chmod +x target/cpio/init
	
	mkdir -p target/cpio/dev
	sudo mknod -m 666 target/cpio/dev/mem c 1 1

	mkdir -p target/cpio/mnt

	mkdir -p target/cpio/etc
	echo "nameserver 8.8.8.8" > target/cpio/etc/resolv.conf

	sudo chown -R root:root target/cpio

	cd target/cpio && find . | cpio -o --format=newc > ../takeoff.cpio
	sudo rm -rf target/cpio

clean:
	cargo clean

build-hello-page:
	truncate -s 64M target/hello-page.ext4
	sudo mkfs.ext4 target/hello-page.ext4

	mkdir -p target/hello-page
	sudo mount target/hello-page.ext4 target/hello-page

	sudo cp hello-page/index.html target/hello-page/index.html

	sudo umount target/hello-page

mount-hello-page:
	mkdir -p target/hello-page
	sudo mount target/hello-page.ext4 target/hello-page

unmount-hello-page:
	sudo umount target/hello-page
	rmdir target/hello-page