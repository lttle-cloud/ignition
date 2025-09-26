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

release-takeoff: build-takeoff
	cp target/takeoff.cpio dist/takeoff.cpio

release-ignitiond-linux:
	cargo build --bin ignitiond --features daemon --release
	cp target/release/ignitiond dist/ignitiond
	strip dist/ignitiond

release-cli-linux:
	cargo build --release --bin lttle --features lovable
	strip target/release/lttle
	mkdir -p dist
	mv target/release/lttle dist/lttle_linux_x86_64

release-cli-darwin:
	cargo build --release --bin lttle --features lovable
	strip target/release/lttle
	mkdir -p dist
	mv target/release/lttle dist/lttle_darwin_aarch64

clean:
	cargo clean