# sc: clang + wasm-ld for C and C++, as one wasm32-wasip1 module.
# `make` builds build/sc.wasm; the first run fetches and builds LLVM twice
# (native tools, then the wasm cross build) and takes about an hour.

B := build
W := $(CURDIR)/build/wasi-sdk-29.0-x86_64-linux
WASI_SDK_URL := https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-29/wasi-sdk-29.0-x86_64-linux.tar.gz
LLVM_REPO := https://github.com/llvm/llvm-project
LLVM_COMMIT := 0d261d1ca552c95a8f007e061c787ac7132fbcbc
LLVM_PATHS := llvm clang lld cmake third-party libunwind libcxx libcxxabi runtimes \
  libc/shared libc/src/__support libc/hdr libc/include/llvm-libc-macros libc/include/llvm-libc-types libc/config
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

ENGINE := sc_entry sc_main sc_inv sc_backend sc_cuts
ENGINE_FLAGS := --target=wasm32-wasip1 --sysroot $(W)/share/wasi-sysroot -D_WASI_EMULATED_MMAN \
  -DLLVM_BUILD_STATIC -D__STDC_CONSTANT_MACROS -D__STDC_FORMAT_MACROS -D__STDC_LIMIT_MACROS \
  -I$(B)/src/clang/include -I$(B)/wasm/tools/clang/include -I$(B)/wasm/include -I$(B)/src/llvm/include \
  -I$(B)/src/lld/include -I$(B)/wasm/tools/lld/include \
  -fvisibility-inlines-hidden -ffunction-sections -fdata-sections -Os -DNDEBUG -std=c++17 \
  -fno-exceptions -fno-rtti -Wall -Wno-unused-parameter

.PHONY: all fetch llvm sysroot engine trim loader native baseline test test-small test-warn test-native learn learn-native checkinv size spike test-proto test-split test-aluno test-checks repl clean
all: $(B)/sc.wasm

# --- sources
$(W)/bin/clang:
	mkdir -p $(B) && curl -sSL $(WASI_SDK_URL) | tar xzf - -C $(B)

$(B)/src/llvm/CMakeLists.txt:
	git init -q $(B)/src && cd $(B)/src && git sparse-checkout set $(LLVM_PATHS) \
	  && git fetch -q --depth 1 $(LLVM_REPO) $(LLVM_COMMIT) && git checkout -q FETCH_HEAD \
	  && git apply $(CURDIR)/llvm/wasi-host.patch $(CURDIR)/llvm/no-veclibs.patch $(CURDIR)/llvm/dylink-weak.patch \
	  $(CURDIR)/llvm/libcxx-locale.patch $(CURDIR)/llvm/incremental-c-scopes.patch \
	  $(CURDIR)/llvm/incremental-stmts.patch $(CURDIR)/llvm/incremental-main-file.patch \
	  $(CURDIR)/llvm/asan-cleanupret.patch

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

# --- the sysroot: wasi-sdk's libc, with libc++, libc++abi and libunwind built
# from the pin with exceptions (-fwasm-exceptions, the standard form, exnref);
# libunwind goes inside libc++abi, which the wasm unwinder calls back. A
# failed check of the hardening calls __libcpp_verbose_abort with its reason
# (enforce), which libsc_rt says in its words; quick_enforce's trap keeps it
# only in the debug info, where wasmtime does not find it (ANALISE.md §32)
SR := $(B)/sysroot
RT_EH := -fwasm-exceptions -mllvm -wasm-use-legacy-eh=false -resource-dir $(W)/lib/clang/21
RT_OPTS := -DCMAKE_BUILD_TYPE=MinSizeRel -DLLVM_COMPILER_CHECKED=ON -DLLVM_ENABLE_RUNTIMES="libcxx;libcxxabi;libunwind" \
  -DCMAKE_C_FLAGS="$(RT_EH)" -DCMAKE_CXX_FLAGS="$(RT_EH)" \
  -DLIBCXX_ABI_VERSION=2 -DLIBCXX_HAS_MUSL_LIBC=ON -DLIBCXX_ENABLE_THREADS=ON -DLIBCXX_HAS_PTHREAD_API=ON \
  -DLIBCXX_ENABLE_SHARED=ON -DLIBCXX_ENABLE_STATIC=ON -DLIBCXX_ENABLE_EXCEPTIONS=ON -DLIBCXX_ENABLE_TIME_ZONE_DATABASE=OFF \
  -DLIBCXX_ENABLE_ABI_LINKER_SCRIPT=OFF -DLIBCXX_CXX_ABI=libcxxabi -DLIBCXX_HARDENING_MODE=fast -DLIBCXX_ASSERTION_SEMANTIC=enforce \
  -DLIBCXX_INCLUDE_TESTS=OFF -DLIBCXX_INCLUDE_BENCHMARKS=OFF -DLIBCXX_ENABLE_STATIC_ABI_LIBRARY=OFF -DLIBCXX_INSTALL_MODULES=ON \
  -DLIBCXXABI_ENABLE_EXCEPTIONS=ON -DLIBCXXABI_ENABLE_SHARED=ON -DLIBCXXABI_ENABLE_THREADS=ON -DLIBCXXABI_HAS_PTHREAD_API=ON \
  -DLIBCXXABI_USE_LLVM_UNWINDER=ON -DLIBCXXABI_ENABLE_STATIC_UNWINDER=ON -DLIBCXXABI_SILENT_TERMINATE=OFF \
  -DLIBCXXABI_STATICALLY_LINK_UNWINDER_IN_SHARED_LIBRARY=ON -DLIBCXXABI_STATICALLY_LINK_UNWINDER_IN_STATIC_LIBRARY=ON \
  -DLIBCXXABI_INCLUDE_TESTS=OFF -DLIBCXXABI_ENABLE_ASSERTIONS=OFF \
  -DLIBUNWIND_ENABLE_SHARED=OFF -DLIBUNWIND_ENABLE_THREADS=ON -DLIBUNWIND_INCLUDE_TESTS=OFF -DLIBUNWIND_USE_COMPILER_RT=OFF \
  -DLIBUNWIND_HIDE_SYMBOLS=OFF -DLIBUNWIND_ENABLE_ASSERTIONS=OFF \
  -DLIBUNWIND_ADDITIONAL_COMPILE_FLAGS="-fvisibility=default;-fdeclspec;-Wno-ignored-attributes" \
  -DLIBCXX_INSTALL_INCLUDE_DIR=include/wasm32-wasip1/c++/v1 -DLIBCXX_INSTALL_INCLUDE_TARGET_DIR=include/wasm32-wasip1/c++/v1 \
  -DLIBCXX_INSTALL_LIBRARY_DIR=lib/wasm32-wasip1 -DLIBCXXABI_INSTALL_LIBRARY_DIR=lib/wasm32-wasip1 \
  -DLIBUNWIND_INSTALL_LIBRARY_DIR=lib/wasm32-wasip1 -DLIBUNWIND_INSTALL_INCLUDE_DIR=include/wasm32-wasip1 \
  -DLIBCXX_INSTALL_MODULES_DIR=share/libc++/v1

$(B)/Toolchain-runtimes.cmake: llvm/runtimes.cmake.in
	sed 's#@WASI_SDK@#$(W)#g; s#@NATIVE@#$(CURDIR)/$(B)/native#g' $< > $@

$(SR)/lib/wasm32-wasip1/libc++.so: $(B)/Toolchain-runtimes.cmake | $(B)/native/bin/wasm-ld $(W)/bin/clang
	cmake -G Ninja -S $(B)/src/runtimes -B $(B)/runtimes -Wno-dev -DCMAKE_TOOLCHAIN_FILE=$(CURDIR)/$< $(RT_OPTS)
	ninja -C $(B)/runtimes cxx cxxabi unwind
	rm -rf $(SR) && mkdir -p $(SR)/include $(SR)/lib
	cp -a $(W)/share/wasi-sysroot/include/wasm32-wasip1 $(SR)/include/ && rm -rf $(SR)/include/wasm32-wasip1/c++
	cp -a $(W)/share/wasi-sysroot/lib/wasm32-wasip1 $(SR)/lib/ && rm -f $(SR)/lib/wasm32-wasip1/libc++*
	cmake --install $(B)/runtimes --prefix $(CURDIR)/$(SR) >/dev/null

# libsc_rt: the handlers of UBSan's checks and ASan's runtime (rt/), for the
# levels on and strict of --safety; shared for the REPL, static for the
# corpus's programs
RT := $(SR)/lib/wasm32-wasip1/libsc_rt
RT_OBJS := $(B)/rt/sc_rt.o $(B)/rt/sc_asan.o
$(B)/rt/%.o: rt/%.c | $(SR)/lib/wasm32-wasip1/libc++.so
	@mkdir -p $(@D)
	$(N)/bin/clang --target=wasm32-wasip1 --sysroot $(SR) -fPIC -fvisibility=default -O2 -Wall -Wextra -Wno-unused-parameter -c $< -o $@

$(RT).a: $(RT_OBJS)
	rm -f $@ && $(W)/bin/llvm-ar rcs $@ $^

$(RT).so: $(RT_OBJS)
	$(N)/bin/wasm-ld -shared --experimental-pic --unresolved-symbols=import-dynamic $^ -L$(SR)/lib/wasm32-wasip1 -lc -o $@

sysroot: $(SR)/lib/wasm32-wasip1/libc++.so $(RT).a $(RT).so

# --- the engine
$(B)/engine/%.o: engine/%.cpp engine/sc.h engine/sc_inv.h | $(B)/wasm/lib/libclangSema.a
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

# --- the native engine: the same engine and cuts over a host build of LLVM
# from the patched sources (build/native, for the tools, is not cut)
HOST_OPTS := $(LLVM_OPTS) -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ -DLLVM_USE_LINKER=lld \
  -DLLVM_ENABLE_PIC=OFF -DLLVM_ENABLE_THREADS=OFF -DLLVM_DEFAULT_TARGET_TRIPLE=wasm32-wasip1 \
  -DLLVM_TABLEGEN=$(CURDIR)/$(B)/native/bin/llvm-tblgen -DCLANG_TABLEGEN=$(CURDIR)/$(B)/native/bin/clang-tblgen \
  -DLLVM_BUILD_TOOLS=OFF -DLLVM_INCLUDE_UTILS=OFF -DLLVM_BUILD_UTILS=OFF -DLLVM_INCLUDE_RUNTIMES=OFF \
  -DCLANG_INCLUDE_TESTS=OFF -DCLANG_BUILD_TOOLS=OFF -DLLD_BUILD_TOOLS=OFF -DDEFAULT_SYSROOT=/usr -DCLANG_RESOURCE_DIR=/usr
HOST_FLAGS := -D_GNU_SOURCE -D__STDC_CONSTANT_MACROS -D__STDC_FORMAT_MACROS -D__STDC_LIMIT_MACROS \
  -I$(B)/src/clang/include -I$(B)/host/tools/clang/include -I$(B)/host/include -I$(B)/src/llvm/include \
  -I$(B)/src/lld/include -I$(B)/host/tools/lld/include \
  -fvisibility-inlines-hidden -ffunction-sections -fdata-sections -Os -DNDEBUG -std=c++17 \
  -fno-exceptions -fno-rtti -Wall -Wno-unused-parameter

$(B)/host/lib/libclangSema.a: | $(B)/native/bin/wasm-ld
	cmake -G Ninja -S $(B)/src/llvm -B $(B)/host $(HOST_OPTS)
	ninja -C $(B)/host clang lld

$(B)/engine-host/%.o: engine/%.cpp engine/sc.h engine/sc_inv.h engine/sc_session.h | $(B)/host/lib/libclangSema.a
	@mkdir -p $(@D)
	clang++ $(HOST_FLAGS) -c $< -o $@

$(B)/trim-host/Intrinsics.cpp.o: scripts/trim.py scripts/trim_intrinsics.py | $(B)/host/lib/libclangSema.a
	scripts/trim.py --native

$(B)/sc-host-cut: $(ENGINE:%=$(B)/engine-host/%.o) $(B)/trim-host/Intrinsics.cpp.o scripts/link.py \
    cuts/objects.txt cuts/symbols.txt cuts/symbols.used cuts/symbols.host.used cuts/o0init.txt
	scripts/link.py --native $@

$(B)/sc: $(B)/sc-host-cut
	llvm-strip -o $@ $<

$(B)/sc-host-full: $(ENGINE:%=$(B)/engine-host/%.o) scripts/link.py
	scripts/link.py --native --full $@

native: $(B)/sc

# --- PLANO-REPL.md phase 0: the two candidates for REPL cells (test/spike.py),
# on the uncut native engine
SPIKE := $(ENGINE) sc_session sc_spike
$(B)/engine-spike/%.o: engine/%.cpp engine/sc.h engine/sc_inv.h engine/sc_session.h | $(B)/host/lib/libclangSema.a
	@mkdir -p $(@D)
	clang++ $(HOST_FLAGS) -DSC_SPIKE -c $< -o $@

$(B)/sc-spike: $(SPIKE:%=$(B)/engine-spike/%.o) scripts/link.py
	SC_ENGINE=$(CURDIR)/$(B)/engine-spike SC_LINKDIR=$(CURDIR)/$(B)/link-spike \
	  SC_EXTRA="$(CURDIR)/$(B)/engine-spike/sc_session.o $(CURDIR)/$(B)/engine-spike/sc_spike.o" \
	  scripts/link.py --native --full $@

spike: $(B)/sc-spike $(SCLOAD)
	test/spike.py

# --- the engine's API for Rust (proto/, the sc-proto crate): the uncut native
# engine with the session (sc.h), as the link arguments for proto/build.rs
SESSION := $(ENGINE) sc_session sc_api
$(B)/link-session/engine.args: $(SESSION:%=$(B)/engine-host/%.o) scripts/link.py
	SC_LINKDIR=$(CURDIR)/$(B)/link-session \
	  SC_EXTRA="$(CURDIR)/$(B)/engine-host/sc_session.o $(CURDIR)/$(B)/engine-host/sc_api.o" \
	  scripts/link.py --native --full --emit-link $@ $(B)/link-session/engine

test-proto: $(B)/link-session/engine.args $(SCLOAD) $(SR)/lib/wasm32-wasip1/libc++.so
	SC_LINK_ARGS=$(CURDIR)/$< cargo test --locked --manifest-path proto/Cargo.toml --features engine \
	  --target-dir $(B)/proto

# the same engine with the REPL's protocol (proto/src/bin/sc-engine.rs): the
# child (session) and the test of the REPL's side (repl), PLANO-REPL.md phase 2
$(B)/sc-engine-full: $(B)/link-session/engine.args $(wildcard proto/*.toml proto/*.lock proto/*.rs proto/src/*.rs proto/src/bin/*.rs)
	SC_LINK_ARGS=$(CURDIR)/$< cargo build --release --locked --manifest-path proto/Cargo.toml --features engine \
	  --target-dir $(B)/proto --bin sc-engine
	cp $(B)/proto/release/sc-engine $@

# the corpus's programs as REPL cells against the programs whole (pic-)
test-split: $(B)/sc-engine-full $(SCLOAD) $(B)/res/native-base/summary.json
	test/split.py

# --- the REPL's loader: runs the shared objects (native, wasmtime). The same
# crate has sc, the REPL (a prototype of phase 6b: its compiler is the child
# sc-engine-full session, with the arguments in build/repl)
SCLOAD := $(B)/loader/release/scload
SC_REPL := $(B)/loader/release/sc
$(SCLOAD): loader/Cargo.toml loader/Cargo.lock loader/src/lib.rs loader/src/main.rs
	cargo build --release --locked --manifest-path loader/Cargo.toml --target-dir $(B)/loader --bin scload
$(SC_REPL): loader/Cargo.toml loader/Cargo.lock $(shell find loader/src -name '*.rs') \
            $(wildcard proto/*.toml proto/*.rs proto/src/*.rs) engine/sc.h
	cargo build --release --locked --manifest-path loader/Cargo.toml --target-dir $(B)/loader --features repl --bin sc

loader: $(SCLOAD)

# a session's arguments: <lang>-<safety level>[-file].args, -file for `sc FILE`
REPL_ARGS := $(foreach l,c c++,$(foreach s,off on strict,$(B)/repl/$l-$s.args $(B)/repl/$l-$s-file.args))
$(B)/repl/%.args: scripts/flags.py
	mkdir -p $(B)/repl
	scripts/flags.py --session $(firstword $(subst -, ,$*)) $(CURDIR)/$(B) $(wordlist 2,3,$(subst -, ,$*)) > $@

repl: $(SC_REPL) $(B)/sc-engine-full $(REPL_ARGS) $(SR)/lib/wasm32-wasip1/libc++.so $(RT).so

# a student's correct code at each level of --safety (PLANO-VERIFICACOES.md §9)
test-aluno: repl
	test/aluno.py

test-checks: repl
	test/checks.py

# --- tests: the corpus against the uncut engine
$(B)/res/base/summary.json: $(B)/sc-full.wasm test/corpus.py scripts/flags.py | $(CORPUS) $(SCLOAD) $(SR)/lib/wasm32-wasip1/libc++.so $(RT).a
	test/corpus.py $< $(B)/res/base

# the same with every warning on, so the cuts keep the code of all warnings
$(B)/res/wev-base/summary.json: $(B)/sc-full.wasm test/corpus.py scripts/flags.py | $(CORPUS) $(SCLOAD) $(SR)/lib/wasm32-wasip1/libc++.so $(RT).a
	WFLAGS=-Weverything test/corpus.py $< $(B)/res/wev-base

baseline: $(B)/res/base/summary.json

test: $(B)/sc.wasm $(B)/res/base/summary.json $(SCLOAD)
	test/corpus.py $(B)/sc.wasm $(B)/res/sc $(B)/res/base/summary.json

test-small: $(B)/sc.wasm $(B)/res/base/summary.json $(SCLOAD)
	CORPUS=small test/corpus.py $(B)/sc.wasm $(B)/res/sc-small $(B)/res/base/summary.json

# the native engine against the uncut native engine (sc.wasm and the native
# engine differ where the host does: stack depth, memory, the sandbox)
$(B)/res/native-base/summary.json: $(B)/sc-host-full test/corpus.py scripts/flags.py | $(CORPUS) $(SCLOAD) $(SR)/lib/wasm32-wasip1/libc++.so $(RT).a
	test/corpus.py $< $(B)/res/native-base

$(B)/res/native-wev-base/summary.json: $(B)/sc-host-full test/corpus.py scripts/flags.py | $(CORPUS) $(SCLOAD) $(SR)/lib/wasm32-wasip1/libc++.so $(RT).a
	WFLAGS=-Weverything test/corpus.py $< $(B)/res/native-wev-base

test-native: $(B)/sc $(B)/res/native-base/summary.json $(SCLOAD)
	test/corpus.py $(B)/sc $(B)/res/native $(B)/res/native-base/summary.json

test-warn: $(B)/sc.wasm $(B)/res/wev-base/summary.json $(SCLOAD)
	WFLAGS=-Weverything test/corpus.py $(B)/sc.wasm $(B)/res/wev $(B)/res/wev-base/summary.json

# rebuild what the cuts keep from the corpus with -Weverything (after an LLVM
# bump or new cuts)
learn: $(ENGINE:%=$(B)/engine/%.o) $(B)/trim/Intrinsics.cpp.obj $(B)/res/wev-base/summary.json
	WFLAGS=-Weverything SC_BASE=$(CURDIR)/$(B)/res/wev-base/summary.json scripts/link.py --learn $(B)/sc-cut.wasm

# what only the native engine calls (cuts/symbols.host.used), after learn
learn-native: $(ENGINE:%=$(B)/engine-host/%.o) $(B)/trim-host/Intrinsics.cpp.o $(B)/res/native-wev-base/summary.json
	WFLAGS=-Weverything SC_BASE=$(CURDIR)/$(B)/res/native-wev-base/summary.json scripts/link.py --native --learn $(B)/sc-host-cut

# the invocation built by engine/sc_inv.cpp against clang's option parser (native)
N := $(B)/native
CHECKINV_FLAGS := -DNDEBUG -std=c++17 -fno-exceptions -fno-rtti -O1 -Iengine \
  -I$(B)/src/clang/include -I$(N)/tools/clang/include -I$(N)/include -I$(B)/src/llvm/include
$(B)/checkinv: test/checkinv.cpp engine/sc_inv.cpp engine/sc_inv.h | $(N)/bin/wasm-ld
	clang++ $(CHECKINV_FLAGS) test/checkinv.cpp engine/sc_inv.cpp -o $@ \
	  -Wl,--start-group $(N)/lib/libclang*.a $(N)/lib/libLLVM*.a -Wl,--end-group -lpthread

checkinv: $(B)/checkinv
	$< $$(scripts/flags.py) -o - /c/x.c
	$< $$(scripts/flags.py gnu17) -o - /c/x.c
	$< $$(scripts/flags.py c++) -o - /c/x.cpp
	$< $$(scripts/flags.py pic) -o - /c/x.c
	$< $$(scripts/flags.py c++ pic) -o - /c/x.cpp
	$< $$(scripts/flags.py c++ pcm) -o - /c/std.cppm
	$< $$(scripts/flags.py c++ mod) -o - /c/x.cpp
	$< $$(scripts/flags.py c++ pic mod) -o - /c/x.cpp
	for l in off on strict; do \
	  $< $$(scripts/flags.py pic $$l) -o - /c/x.c && \
	  $< $$(scripts/flags.py c++ pic $$l) -o - /c/x.cpp && \
	  $< $$(scripts/flags.py c++ pic mod $$l) -o - /c/x.cpp || exit 1; \
	done
	$< $$(scripts/flags.py pic on g) -o - /c/x.c
	$< $$(scripts/flags.py c++ pic mod on g) -o - /c/x.cpp
	$< $$(scripts/flags.py c++ pcm on) -o - /c/std.cppm

size: $(B)/sc.wasm
	@for f in $(wildcard $(B)/sc.wasm $(B)/sc); do printf '%s: %s raw, %s gzip, %s zstd\n' $$f $$(stat -c %s $$f) \
	  $$(gzip -9 -c $$f | wc -c) $$(zstd -19 -c $$f 2>/dev/null | wc -c); done

clean:
	rm -rf $(B)/engine $(B)/engine-host $(B)/engine-spike $(B)/link-spike $(B)/sc-spike $(B)/link-session $(B)/proto $(B)/sc-engine-full $(B)/res/split $(B)/res/spike $(B)/trim $(B)/trim-host $(B)/link-* $(B)/sc*.wasm $(B)/sc $(B)/sc-host-* $(B)/res/sc* $(B)/res/native
