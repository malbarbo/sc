# sc: clang + wasm-ld for C and C++, as one wasm32-wasip1 module.
# `make` builds build/sc.wasm; the first run fetches and builds LLVM twice
# (native tools, then the wasm cross build) and takes about an hour.

B := build
W := $(CURDIR)/build/wasi-sdk-29.0-x86_64-linux
WASI_SDK_URL := https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-29/wasi-sdk-29.0-x86_64-linux.tar.gz
LLVM_REPO := https://github.com/llvm/llvm-project
LLVM_COMMIT := 5fe235b9862f7fb87012eb06bd71890fa5f41736
LLVM_PATHS := llvm clang lld cmake third-party libunwind/include libcxx/src/filesystem
CTS_REPO := https://github.com/c-testsuite/c-testsuite
CTS_COMMIT := 5c7275656d751de0e68b2d340a95b5681858ed07
GCC_REPO := https://github.com/gcc-mirror/gcc
GCC_COMMIT := ca5cffec066fb8a7edd13dd12792505739381b42

LLVM_OPTS := -DCMAKE_BUILD_TYPE=MinSizeRel -DLLVM_ENABLE_ASSERTIONS=OFF \
  -DLLVM_TARGETS_TO_BUILD=WebAssembly -DLLVM_ENABLE_PROJECTS="clang;lld" \
  -DLLVM_INCLUDE_TESTS=OFF -DLLVM_INCLUDE_EXAMPLES=OFF -DLLVM_INCLUDE_BENCHMARKS=OFF -DLLVM_INCLUDE_DOCS=OFF \
  -DCLANG_ENABLE_STATIC_ANALYZER=OFF -DCLANG_ENABLE_OBJC_REWRITER=OFF -DCLANG_PLUGIN_SUPPORT=OFF -DLLVM_ENABLE_PLUGINS=OFF \
  -DLLVM_ENABLE_ZLIB=OFF -DLLVM_ENABLE_ZSTD=OFF -DLLVM_ENABLE_LIBXML2=OFF -DLLVM_ENABLE_LIBEDIT=OFF \
  -DLLVM_ENABLE_BACKTRACES=OFF -DLLVM_ENABLE_CRASH_OVERRIDES=OFF
WASM_OPTS := -DCMAKE_TOOLCHAIN_FILE=$(CURDIR)/$(B)/Toolchain-WASI-LLVM.cmake -DLLVM_NATIVE_TOOL_DIR=$(CURDIR)/$(B)/native/bin \
  -DLLVM_BUILD_SHARED_LIBS=OFF -DLLVM_ENABLE_PIC=OFF -DLLVM_BUILD_STATIC=ON -DLLVM_ENABLE_THREADS=OFF \
  -DLLVM_BUILD_RUNTIME=OFF -DLLVM_BUILD_TOOLS=OFF -DLLVM_INCLUDE_UTILS=OFF -DLLVM_BUILD_UTILS=OFF \
  -DLLVM_INCLUDE_RUNTIMES=OFF -DLLVM_DEFAULT_TARGET_TRIPLE=wasm32-wasip1 \
  -DCLANG_INCLUDE_TESTS=OFF -DCLANG_BUILD_TOOLS=OFF -DCLANG_BUILD_EXAMPLES=OFF -DCLANG_INCLUDE_DOCS=OFF \
  -DLLD_BUILD_TOOLS=OFF -DDEFAULT_SYSROOT=/usr -DCLANG_RESOURCE_DIR=/usr

ENGINE := sc_main sc_inv sc_backend sc_cuts
ENGINE_FLAGS := --target=wasm32-wasip1 --sysroot $(W)/share/wasi-sysroot -D_WASI_EMULATED_MMAN \
  -DLLVM_BUILD_STATIC -D__STDC_CONSTANT_MACROS -D__STDC_FORMAT_MACROS -D__STDC_LIMIT_MACROS \
  -I$(B)/src/clang/include -I$(B)/wasm/tools/clang/include -I$(B)/wasm/include -I$(B)/src/llvm/include \
  -I$(B)/src/lld/include -I$(B)/wasm/tools/lld/include \
  -fvisibility-inlines-hidden -ffunction-sections -fdata-sections -Os -DNDEBUG -std=c++17 \
  -fno-exceptions -fno-rtti -Wall -Wno-unused-parameter

.PHONY: all fetch llvm engine trim loader baseline test test-small test-warn learn checkinv size clean
all: $(B)/sc.wasm

# --- sources
$(W)/bin/clang:
	mkdir -p $(B) && curl -sSL $(WASI_SDK_URL) | tar xzf - -C $(B)

$(B)/src/llvm/CMakeLists.txt:
	git init -q $(B)/src && cd $(B)/src && git sparse-checkout set $(LLVM_PATHS) \
	  && git fetch -q --depth 1 $(LLVM_REPO) $(LLVM_COMMIT) && git checkout -q FETCH_HEAD \
	  && git apply $(CURDIR)/llvm/wasi-host.patch $(CURDIR)/llvm/no-veclibs.patch $(CURDIR)/llvm/dylink-weak.patch

$(B)/c-testsuite/.git:
	git init -q $(B)/c-testsuite && cd $(B)/c-testsuite \
	  && git fetch -q --depth 1 $(CTS_REPO) $(CTS_COMMIT) && git checkout -q FETCH_HEAD

$(B)/gcc-ts/.git:
	git init -q $(B)/gcc-ts && cd $(B)/gcc-ts \
	  && git sparse-checkout set gcc/testsuite/gcc.c-torture gcc/testsuite/gcc.dg/torture \
	  && git fetch -q --depth 1 --filter=blob:none $(GCC_REPO) $(GCC_COMMIT) && git checkout -q FETCH_HEAD

CORPUS := $(B)/c-testsuite/.git $(B)/gcc-ts/.git $(B)/src/llvm/CMakeLists.txt
fetch: $(W)/bin/clang $(CORPUS)

# --- LLVM: native tools (tablegen, wasm-ld, llc), then the wasm libraries
$(B)/native/bin/wasm-ld: | $(B)/src/llvm/CMakeLists.txt
	cmake -G Ninja -S $(B)/src/llvm -B $(B)/native $(LLVM_OPTS) \
	  -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ -DLLVM_USE_LINKER=lld
	ninja -C $(B)/native

$(B)/Toolchain-WASI-LLVM.cmake: llvm/toolchain.cmake.in
	sed 's#@WASI_SDK@#$(W)#g' $< > $@

$(B)/wasm/lib/libclangSema.a: | $(B)/native/bin/wasm-ld $(B)/Toolchain-WASI-LLVM.cmake $(W)/bin/clang
	cmake -G Ninja -S $(B)/src/llvm -B $(B)/wasm $(LLVM_OPTS) $(WASM_OPTS)
	ninja -C $(B)/wasm clang lld

llvm: $(B)/wasm/lib/libclangSema.a

# --- the engine
$(B)/engine/%.o: engine/%.cpp engine/sc_inv.h | $(B)/wasm/lib/libclangSema.a
	@mkdir -p $(@D)
	$(W)/bin/clang++ $(ENGINE_FLAGS) -c $< -o $@

engine: $(ENGINE:%=$(B)/engine/%.o)

$(B)/trim/Intrinsics.cpp.obj: scripts/trim.py scripts/trim_intrinsics.py | $(B)/wasm/lib/libclangSema.a
	scripts/trim.py

trim: $(B)/trim/Intrinsics.cpp.obj

$(B)/sc-full.wasm: $(ENGINE:%=$(B)/engine/%.o) scripts/link.py
	scripts/link.py --full $@

$(B)/sc-cut.wasm: $(ENGINE:%=$(B)/engine/%.o) $(B)/trim/Intrinsics.cpp.obj scripts/link.py \
    cuts/objects.txt cuts/symbols.txt cuts/symbols.used cuts/o0init.txt
	scripts/link.py $@

$(B)/sc.wasm: $(B)/sc-cut.wasm
	wasm-opt -Oz --strip-debug --strip-producers $< -o $@

# --- the REPL's loader: runs the shared objects (native, wasmtime)
SCLOAD := $(B)/loader/release/scload
$(SCLOAD): loader/Cargo.toml loader/Cargo.lock loader/src/main.rs
	cargo build --release --locked --manifest-path loader/Cargo.toml --target-dir $(B)/loader

loader: $(SCLOAD)

# --- tests: the corpus against the uncut engine
$(B)/res/base/summary.json: $(B)/sc-full.wasm test/corpus.py | $(CORPUS) $(SCLOAD)
	test/corpus.py $< $(B)/res/base

# the same with every warning on, so the cuts keep the code of all warnings
$(B)/res/wev-base/summary.json: $(B)/sc-full.wasm test/corpus.py | $(CORPUS) $(SCLOAD)
	WFLAGS=-Weverything test/corpus.py $< $(B)/res/wev-base

baseline: $(B)/res/base/summary.json

test: $(B)/sc.wasm $(B)/res/base/summary.json $(SCLOAD)
	test/corpus.py $(B)/sc.wasm $(B)/res/sc $(B)/res/base/summary.json

test-small: $(B)/sc.wasm $(B)/res/base/summary.json $(SCLOAD)
	CORPUS=small test/corpus.py $(B)/sc.wasm $(B)/res/sc-small $(B)/res/base/summary.json

test-warn: $(B)/sc.wasm $(B)/res/wev-base/summary.json $(SCLOAD)
	WFLAGS=-Weverything test/corpus.py $(B)/sc.wasm $(B)/res/wev $(B)/res/wev-base/summary.json

# rebuild what the cuts keep from the corpus with -Weverything (after an LLVM
# bump or new cuts)
learn: $(ENGINE:%=$(B)/engine/%.o) $(B)/trim/Intrinsics.cpp.obj $(B)/res/wev-base/summary.json
	WFLAGS=-Weverything SC_BASE=$(CURDIR)/$(B)/res/wev-base/summary.json scripts/link.py --learn $(B)/sc-cut.wasm

# the invocation built by engine/sc_inv.cpp against clang's option parser (native)
N := $(B)/native
CHECKINV_FLAGS := -DNDEBUG -std=c++17 -fno-exceptions -fno-rtti -O1 -Iengine \
  -I$(B)/src/clang/include -I$(N)/tools/clang/include -I$(N)/include -I$(B)/src/llvm/include
$(B)/checkinv: test/checkinv.cpp engine/sc_inv.cpp engine/sc_inv.h | $(N)/bin/wasm-ld
	clang++ $(CHECKINV_FLAGS) test/checkinv.cpp engine/sc_inv.cpp -o $@ \
	  -Wl,--start-group $(N)/lib/libclang*.a $(N)/lib/libLLVM*.a -Wl,--end-group -lpthread

checkinv: $(B)/checkinv
	$< $$(test/corpus.py --flags) -o - /c/x.c
	$< $$(test/corpus.py --flags c++) -o - /c/x.cpp
	$< $$(test/corpus.py --flags pic) -o - /c/x.c
	$< $$(test/corpus.py --flags c++ pic) -o - /c/x.cpp
	$< $$(test/corpus.py --flags c++ pcm) -o - /c/std.cppm
	$< $$(test/corpus.py --flags c++ mod) -o - /c/x.cpp
	$< $$(test/corpus.py --flags c++ pic mod) -o - /c/x.cpp

size: $(B)/sc.wasm
	@printf '%s: %s raw, %s gzip, %s zstd\n' $< $$(stat -c %s $<) \
	  $$(gzip -9 -c $< | wc -c) $$(zstd -19 -c $< 2>/dev/null | wc -c)

clean:
	rm -rf $(B)/engine $(B)/trim $(B)/link-* $(B)/sc*.wasm $(B)/res/sc*
