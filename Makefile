takeoff:
	cd src/takeoff && RUSTFLAGS="-C link-arg=-nostartfiles -C target-feature=+crt-static" cargo build --bin takeoff --release --target x86_64-unknown-linux-musl
	
	mkdir -p target/cpio

	mkdir -p target/cpio/sbin
	cp target/x86_64-unknown-linux-musl/release/takeoff target/cpio/sbin/init

	mkdir -p target/cpio/dev
	sudo mknod -m 666 target/cpio/dev/null c 1 3
	sudo mknod -m 666 target/cpio/dev/zero c 1 5
	sudo mknod -m 666 target/cpio/dev/mem c 1 1

	sudo chown -R root:root target/cpio

	cd target/cpio && find . | cpio -o --format=newc > ../takeoff.cpio
	sudo rm -rf target/cpio

clean:
	cargo clean