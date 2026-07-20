.PHONY: help \
        rust-build rust-test rust-clean \
        go-build go-test go-clean \
        cpp-build cpp-test cpp-clean \
        test build clean

CPP_DIR    := cpp
CPP_BUILD  := $(CPP_DIR)/build
CXX        ?= clang++
CXXFLAGS   ?= -std=c++17 -O2

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
	@echo "C++ targets:"
	@echo "  make cpp-build     - Compile C++ library and tests"
	@echo "  make cpp-test      - Build and run C++ tests"
	@echo "  make cpp-clean     - Clean C++ build artifacts"
	@echo ""
	@echo "Combined targets:"
	@echo "  make build         - Build Rust, Go, and C++"
	@echo "  make test          - Test Rust, Go, and C++"
	@echo "  make clean         - Clean Rust, Go, and C++"
	@echo "  make help          - Display this help message"

# ─── Rust ────────────────────────────────────────────────────────────────────
rust-build:
	cd rust && cargo build

rust-test:
	cd rust && cargo test --test chunk_parser_tests

rust-clean:
	cd rust && cargo clean

# ─── Go ──────────────────────────────────────────────────────────────────────
go-build:
	cd go && go build ./...

go-test:
	cd go && go test ./...

go-clean:
	cd go && go clean

# ─── C++ ─────────────────────────────────────────────────────────────────────
# Detect whether cmake is available; if not, fall back to a direct clang++ build.
CMAKE := $(shell command -v cmake 2>/dev/null)

cpp-build:
ifdef CMAKE
	mkdir -p $(CPP_BUILD)
	cd $(CPP_BUILD) && cmake .. -DCMAKE_BUILD_TYPE=Release
	$(MAKE) -C $(CPP_BUILD)
else
	@echo "cmake not found – building directly with $(CXX)"
	mkdir -p $(CPP_BUILD)
	$(CXX) $(CXXFLAGS) \
	  -I$(CPP_DIR)/include \
	  -I$(CPP_DIR)/third_party \
	  $(CPP_DIR)/src/parser.cpp \
	  $(CPP_DIR)/src/chunk_parser.cpp \
	  $(CPP_DIR)/tests/chunk_parser_tests.cpp \
	  -o $(CPP_BUILD)/chunk_parser_tests
endif

cpp-test: cpp-build
ifdef CMAKE
	cd $(CPP_BUILD) && ctest --output-on-failure
else
	$(CPP_BUILD)/chunk_parser_tests
endif

cpp-clean:
	rm -rf $(CPP_BUILD)

# ─── Combined ────────────────────────────────────────────────────────────────
build: rust-build go-build cpp-build

test: rust-test go-test cpp-test

clean: rust-clean go-clean cpp-clean
