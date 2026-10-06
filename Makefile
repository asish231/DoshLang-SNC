CC ?= clang
# Prefer rustup's cargo if not already on PATH:
export PATH := $(HOME)/.cargo/bin:$(PATH)

.PHONY: all snc test clean run example

all: snc

# Build the release compiler into ./snc
snc:
	cd compiler && cargo build --release
	cp compiler/target/release/snc ./snc

# Unit + integration tests (cargo test), then a hello smoke run
test: snc
	cd compiler && cargo test
	./snc examples/hello_world.sn -o /tmp/snc_hello
	/tmp/snc_hello

run: snc
	./snc examples/hello_world.sn -o /tmp/snc_run
	/tmp/snc_run

example: run

clean:
	cd compiler && cargo clean
	rm -f snc snc.exe
