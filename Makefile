.PHONY: help rust-build rust-test rust-clean go-build go-test go-clean test build clean

help:
	@echo "json-chunk Makefile targets:"
	@echo ""
	@echo "Rust targets:"
	@echo "  make rust-build    - Compile Rust library"
	@echo "  make rust-test     - Run Rust tests"
	@echo "  make rust-clean    - Clean Rust build artifacts"
	@echo ""
	@echo "Go targets:"
	@echo "  make go-build      - Compile Go library"
	@echo "  make go-test       - Run Go tests"
	@echo "  make go-clean      - Clean Go build artifacts"
	@echo ""
	@echo "Combined targets:"
	@echo "  make build         - Build both Rust and Go"
	@echo "  make test          - Test both Rust and Go"
	@echo "  make clean         - Clean both Rust and Go"
	@echo "  make help          - Display this help message"

# Rust targets
rust-build:
	cd rust && cargo build

rust-test:
	cd rust && cargo test --test chunk_parser_tests

rust-clean:
	cd rust && cargo clean

# Go targets
go-build:
	cd go && go build ./...

go-test:
	cd go && go test ./...

go-clean:
	cd go && go clean

# Combined targets
build: rust-build go-build

test: rust-test go-test

clean: rust-clean go-clean
