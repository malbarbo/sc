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

# the Rust crate (Cargo.toml): sc and scload in build/cargo, where cargo
# build puts them too (.cargo/config.toml). Each variant of the engine sc
# links (SC_LINK_ARGS, build.rs) builds in a directory of its own
CARGO := --locked
RUST := Cargo.toml Cargo.lock build.rs $(shell find src -name '*.rs') engine/sc.h
SCLOAD := $(B)/cargo/release/scload
SC_REPL := $(B)/cargo/release/sc
# what the sc binaries embed (build.rs, PLANO-REPL.md §5.4): the arguments
# of a session in the pack's view, the sysroot, its headers and libraries
PACK_ARGS := $(foreach l,c c++,$(foreach s,off on strict,$(B)/pack/repl/$l-$s.args $(B)/pack/repl/$l-$s-file.args))
PACK = $(PACK_ARGS) $(SR)/lib/wasm32-wasip1/libc++.so $(RT).so $(SR)/.setjmp $(IGNORELIST) $(SR)/.ctype

# the engine with the session (sc.h); sc.wasm is a reactor, with its _start
# (PLANO-REPL.md §5.2). scripts/link.py has the same lists
ENGINE := sc_entry sc_main sc_inv sc_backend sc_cuts sc_session sc_api sc_files
WENGINE := $(ENGINE) sc_start
ENGINE_FLAGS := --target=wasm32-wasip1 --sysroot $(W)/share/wasi-sysroot -D_WASI_EMULATED_MMAN \
  -DLLVM_BUILD_STATIC -D__STDC_CONSTANT_MACROS -D__STDC_FORMAT_MACROS -D__STDC_LIMIT_MACROS \
  -I$(B)/src/clang/include -I$(B)/wasm/tools/clang/include -I$(B)/wasm/include -I$(B)/src/llvm/include \
  -I$(B)/src/lld/include -I$(B)/wasm/tools/lld/include \
  -fvisibility-inlines-hidden -ffunction-sections -fdata-sections -Os -DNDEBUG -std=c++17 \
  -fno-exceptions -fno-rtti -Wall -Wno-unused-parameter

.PHONY: all fetch llvm sysroot engine trim loader native baseline test test-small test-warn test-native learn learn-native learn-session learn-session-wasm checkinv size test-proto test-split test-session test-session-warn test-session-wasm test-session-wasm-warn test-split-wasm test-aluno test-checks test-dist repl sc-deps test-loader dist clean
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
	  $(CURDIR)/llvm/asan-cleanupret.patch $(CURDIR)/llvm/pattern-init.patch $(CURDIR)/llvm/repl-outside-function.patch \
	  $(CURDIR)/llvm/incremental-anon-namespace.patch $(CURDIR)/llvm/incremental-input-end.patch \
	  $(CURDIR)/llvm/incremental-access.patch $(CURDIR)/llvm/incremental-remove-decl.patch $(CURDIR)/llvm/lld-files.patch

$(B)/c-testsuite/.git:
	git init -q $(B)/c-testsuite && cd $(B)/c-testsuite \
	  && git fetch -q --depth 1 $(CTS_REPO) $(CTS_COMMIT) && git checkout -q FETCH_HEAD

$(B)/gcc-ts/.git:
	git init -q $(B)/gcc-ts && cd $(B)/gcc-ts \
	  && git sparse-checkout set gcc/testsuite/gcc.c-torture gcc/testsuite/gcc.dg/torture \
	  && git fetch -q --depth 1 --filter=blob:none $(GCC_REPO) $(GCC_COMMIT) && git checkout -q FETCH_HEAD

CORPUS := $(B)/c-testsuite/.git $(B)/gcc-ts/.git $(B)/src/llvm/CMakeLists.txt
fetch: $(W)/bin/clang $(CORPUS)

# --- LLVM: native tools, then the wasm libraries. Of the native tools only
# the tablegens (for the wasm and host builds), clang (the sysroot's) and
# wasm-ld (every link) are used; clang's libraries are checkinv's
$(B)/native/bin/wasm-ld: | $(B)/src/llvm/CMakeLists.txt
	cmake -G Ninja -S $(B)/src/llvm -B $(B)/native $(LLVM_OPTS) \
	  -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ -DLLVM_USE_LINKER=lld
	ninja -C $(B)/native clang lld llvm-tblgen clang-tblgen llvm-min-tblgen

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

# where U-estrito stays off: the sysroot's headers (ANALISE.md §40)
IGNORELIST := $(SR)/share/sc/ignorelist.txt
$(IGNORELIST): rt/ignorelist.txt | $(SR)/lib/wasm32-wasip1/libc++.so
	@mkdir -p $(@D)
	cp $< $@

# musl's isdigit, isalpha... are macros that wrap an unsigned on purpose, in
# the program's code, where the ignorelist does not reach: without them they
# are libc's functions (ANALISE.md §40)
$(SR)/.ctype: | $(SR)/lib/wasm32-wasip1/libc++.so
	sed -i '/^#define isw\?[a-z]*(a) (0 ? isw\?[a-z]*(a) : .*(unsigned)(a)/d' \
	  $(addprefix $(SR)/include/wasm32-wasip1/,ctype.h wchar.h wctype.h)
	touch $@

# libsetjmp.so again from wasi-sdk's libsetjmp.a, with the tag of
# rt/c_longjmp.s; the stamp goes with the sysroot
$(SR)/.setjmp: rt/c_longjmp.s | $(SR)/lib/wasm32-wasip1/libc++.so
	@mkdir -p $(B)/rt
	$(N)/bin/clang --target=wasm32-wasip1 -mexception-handling -c $< -o $(B)/rt/c_longjmp.o
	$(N)/bin/wasm-ld -shared --experimental-pic --unresolved-symbols=import-dynamic $(B)/rt/c_longjmp.o \
	  $(W)/share/wasi-sysroot/lib/wasm32-wasip1/libsetjmp.a -u __wasm_setjmp \
	  $(foreach f,__wasm_setjmp __wasm_setjmp_test __wasm_longjmp,--export=$(f)) \
	  -L$(SR)/lib/wasm32-wasip1 -lc -o $(SR)/lib/wasm32-wasip1/libsetjmp.so
	touch $@

sysroot: $(SR)/lib/wasm32-wasip1/libc++.so $(SR)/.setjmp $(RT).a $(RT).so $(IGNORELIST) $(SR)/.ctype

# --- the engine
$(B)/engine/%.o: engine/%.cpp engine/sc.h engine/sc_inv.h engine/sc_session.h | $(B)/wasm/lib/libclangSema.a
	@mkdir -p $(@D)
	$(W)/bin/clang++ $(ENGINE_FLAGS) -c $< -o $@

engine: $(WENGINE:%=$(B)/engine/%.o)

$(B)/trim/Intrinsics.cpp.obj: scripts/trim.py scripts/trim_intrinsics.py | $(B)/wasm/lib/libclangSema.a
	scripts/trim.py

trim: $(B)/trim/Intrinsics.cpp.obj

$(B)/sc-full.wasm: $(WENGINE:%=$(B)/engine/%.o) scripts/link.py
	scripts/link.py --full $@

$(B)/sc-cut.wasm: $(WENGINE:%=$(B)/engine/%.o) $(B)/trim/Intrinsics.cpp.obj scripts/link.py \
    cuts/objects.txt cuts/symbols.txt cuts/symbols.used cuts/symbols.session.used cuts/o0init.txt
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

# the binary that ships (PLANO-REPL.md §1.1): the REPL with the engine
# linked in. sc --cc1 and sc --ld are the cut engine, the corpus's way in
$(B)/sc: $(SC_REPL)
	llvm-strip -o $@ $<

native: $(B)/sc

# --- the engine's API for Rust (src/engine): the uncut native engine with
# the session (sc.h), as the link arguments for build.rs
$(B)/link-session/engine.args: $(ENGINE:%=$(B)/engine-host/%.o) scripts/link.py
	SC_LINKDIR=$(CURDIR)/$(B)/link-session scripts/link.py --native --full --emit-link $@ $(B)/link-session/engine

test-proto: $(B)/link-session/engine.args $(SCLOAD) $(SR)/lib/wasm32-wasip1/libc++.so
	SC_LINK_ARGS=$(CURDIR)/$< cargo test $(CARGO) --target-dir $(B)/cargo-full --lib --bins --test session

# sc with the same engine: the child (sc --engine) and the test of the
# REPL's side (sc --replay), PLANO-REPL.md phase 2, the base of build/sc's
$(B)/sc-full: $(B)/link-session/engine.args $(RUST) $(PACK)
	SC_LINK_ARGS=$(CURDIR)/$< cargo build --release $(CARGO) --target-dir $(B)/cargo-full --bin sc
	cp $(B)/cargo-full/release/sc $@

# the same, cut (PLANO-REPL.md phase 4): the REPL's engine. What only the
# session calls is in cuts/symbols.session.used (make learn-session)
$(B)/link-session-cut/engine.args: $(ENGINE:%=$(B)/engine-host/%.o) $(B)/trim-host/Intrinsics.cpp.o scripts/link.py \
    cuts/objects.txt cuts/symbols.txt cuts/symbols.used cuts/symbols.host.used cuts/symbols.session.used cuts/o0init.txt
	SC_LINKDIR=$(CURDIR)/$(B)/link-session-cut scripts/link.py --native --emit-link $@ $(B)/link-session-cut/engine

# the corpus's programs as REPL cells against the programs whole (pic-)
test-split: $(B)/sc $(SCLOAD) $(B)/res/native-base/summary.json
	test/split.py $(B)/sc

# the slices of the session (PLANO-REPL.md phase 3, §6): ses-, seq-, crash-,
# mem-; on the uncut engine they are the base of the cut one's
$(B)/res/session-base/summary.json: $(B)/sc-full test/session.py scripts/flags.py | $(SCLOAD) $(B)/res/native-base/summary.json
	test/session.py $< $(@D)

$(B)/res/session-wev-base/summary.json: $(B)/sc-full test/session.py scripts/flags.py | $(SCLOAD) $(B)/res/native-wev-base/summary.json
	WFLAGS=-Weverything SLICES=ses,seq,crash test/session.py $< $(@D)

test-session: $(B)/sc $(SCLOAD) $(B)/res/session-base/summary.json
	test/session.py $(B)/sc $(B)/res/session $(B)/res/session-base/summary.json

test-session-warn: $(B)/sc $(SCLOAD) $(B)/res/session-wev-base/summary.json
	WFLAGS=-Weverything SLICES=ses,seq,crash test/session.py $(B)/sc $(B)/res/session-wev $(B)/res/session-wev-base/summary.json

# --- sc.wasm's session through its exports (PLANO-REPL.md §5.2): the same
# protocol's loop over wasmtime (sc with the feature wasm). <host>.wasm is its
# engine, compiled to <host>.cwasm; -cut has the names, for the cut function
# a trap calls. The slices are checked against the one-shot sc.wasm's bases
SC_WASM := $(B)/cargo-wasm/release/sc
$(SC_WASM): $(RUST) $(PACK)
	cargo build --release $(CARGO) --features wasm --target-dir $(B)/cargo-wasm --bin sc

$(B)/sc-wasm-full: $(SC_WASM) $(B)/sc-full.wasm
$(B)/sc-wasm-cut: $(SC_WASM) $(B)/sc-cut.wasm
$(B)/sc-wasm: $(SC_WASM) $(B)/sc.wasm
$(B)/sc-wasm-full $(B)/sc-wasm-cut $(B)/sc-wasm:
	cp $< $@ && ln -sf $(notdir $(word 2,$^)) $@.wasm && $@ --precompile

$(B)/res/session-wasm-base/summary.json: $(B)/sc-wasm-full test/session.py scripts/flags.py | $(SCLOAD) $(B)/res/base/summary.json
	test/session.py --wasm $< $(@D)

$(B)/res/session-wasm-wev-base/summary.json: $(B)/sc-wasm-full test/session.py scripts/flags.py | $(SCLOAD) $(B)/res/wev-base/summary.json
	WFLAGS=-Weverything SLICES=ses,seq,crash test/session.py --wasm $< $(@D)

test-session-wasm: $(B)/sc-wasm $(SCLOAD) $(B)/res/session-wasm-base/summary.json
	test/session.py --wasm $(B)/sc-wasm $(B)/res/session-wasm $(B)/res/session-wasm-base/summary.json

test-session-wasm-warn: $(B)/sc-wasm $(SCLOAD) $(B)/res/session-wasm-wev-base/summary.json
	WFLAGS=-Weverything SLICES=ses,seq,crash test/session.py --wasm $(B)/sc-wasm $(B)/res/session-wasm-wev \
	  $(B)/res/session-wasm-wev-base/summary.json

test-split-wasm: $(B)/sc-wasm $(SCLOAD) $(B)/res/native-base/summary.json
	test/split.py $(B)/sc-wasm $(B)/res/split-wasm

# --- the REPL's loader, runs the shared objects (native, wasmtime): scload,
# for the batteries, without the engine; and sc, with the cut engine
# (PLANO-REPL.md §1.1, phase 6b), whose compiler is the child sc --engine,
# and what a session reads in the pack it embeds. Without SC_LINK_ARGS, build.rs links
# $(SC_ENGINE_ARGS), as in a cargo build of its own
$(SCLOAD): $(RUST)
	cargo build --release $(CARGO) --no-default-features --bin scload
SC_ENGINE_ARGS := $(B)/link-session-cut/engine.args
$(SC_REPL): $(RUST) $(SC_ENGINE_ARGS) $(PACK)
	cargo build --release $(CARGO) --bin sc

loader: $(SCLOAD)

# the binaries that ship, with the profile that makes them small
# (ANALISE.md §49): build/cargo/dist/scload and sc
dist: $(SC_ENGINE_ARGS) $(PACK)
	cargo build $(CARGO) --profile dist --bins

# what the batteries do not reach: Ctrl-C, many exceptions (tests/), without
# the engine
test-loader:
	cargo test --release $(CARGO) --no-default-features --lib --test signal --test exceptions

# a session's arguments: <lang>-<safety level>[-file].args, -file for `sc
# FILE`, in the pack's view. build.rs puts them in the pack sc embeds
# (PLANO-REPL.md §5.4), with the sysroot, the headers and the libraries
$(B)/pack/repl/%.args: scripts/flags.py
	mkdir -p $(B)/pack/repl
	scripts/flags.py --session $(firstword $(subst -, ,$*)) /sc $(wordlist 2,3,$(subst -, ,$*)) > $@

repl: $(SC_REPL) sc-deps

# what cargo build asks for (build.rs): the engine sc links and what it
# embeds
sc-deps: $(SC_ENGINE_ARGS) $(PACK)

# a student's correct code at each level of --safety (PLANO-VERIFICACOES.md §9)
test-aluno: repl
	test/aluno.py

test-checks: repl
	test/checks.py

# before shipping: the batteries of the REPL on the dist sc (lto, "s",
# panic=abort), against the same bases
test-dist: dist $(SCLOAD) $(B)/res/native-base/summary.json $(B)/res/session-base/summary.json sc-deps
	test/split.py $(B)/cargo/dist/sc $(B)/res/split-dist
	test/session.py $(B)/cargo/dist/sc $(B)/res/session-dist $(B)/res/session-base/summary.json
	test/aluno.py $(B)/cargo/dist/sc
	test/checks.py $(B)/cargo/dist/sc

# --- tests: the corpus against the uncut engine
$(B)/res/base/summary.json: $(B)/sc-full.wasm test/corpus.py scripts/flags.py | $(CORPUS) $(SCLOAD) $(SR)/lib/wasm32-wasip1/libc++.so $(RT).a $(SR)/.setjmp $(IGNORELIST) $(SR)/.ctype
	test/corpus.py $< $(B)/res/base

# the same with every warning on, so the cuts keep the code of all warnings
$(B)/res/wev-base/summary.json: $(B)/sc-full.wasm test/corpus.py scripts/flags.py | $(CORPUS) $(SCLOAD) $(SR)/lib/wasm32-wasip1/libc++.so $(RT).a $(SR)/.setjmp $(IGNORELIST) $(SR)/.ctype
	WFLAGS=-Weverything test/corpus.py $< $(B)/res/wev-base

baseline: $(B)/res/base/summary.json

test: $(B)/sc.wasm $(B)/res/base/summary.json $(SCLOAD)
	test/corpus.py $(B)/sc.wasm $(B)/res/sc $(B)/res/base/summary.json

test-small: $(B)/sc.wasm $(B)/res/base/summary.json $(SCLOAD)
	CORPUS=small test/corpus.py $(B)/sc.wasm $(B)/res/sc-small $(B)/res/base/summary.json

# the native engine against the uncut native engine (sc.wasm and the native
# engine differ where the host does: stack depth, memory, the sandbox)
$(B)/res/native-base/summary.json: $(B)/sc-full test/corpus.py scripts/flags.py | $(CORPUS) $(SCLOAD) $(SR)/lib/wasm32-wasip1/libc++.so $(RT).a $(SR)/.setjmp $(IGNORELIST) $(SR)/.ctype
	test/corpus.py $< $(B)/res/native-base

$(B)/res/native-wev-base/summary.json: $(B)/sc-full test/corpus.py scripts/flags.py | $(CORPUS) $(SCLOAD) $(SR)/lib/wasm32-wasip1/libc++.so $(RT).a $(SR)/.setjmp $(IGNORELIST) $(SR)/.ctype
	WFLAGS=-Weverything test/corpus.py $< $(B)/res/native-wev-base

test-native: $(B)/sc $(B)/res/native-base/summary.json $(SCLOAD)
	test/corpus.py $(B)/sc $(B)/res/native $(B)/res/native-base/summary.json

test-warn: $(B)/sc.wasm $(B)/res/wev-base/summary.json $(SCLOAD)
	WFLAGS=-Weverything test/corpus.py $(B)/sc.wasm $(B)/res/wev $(B)/res/wev-base/summary.json

# rebuild what the cuts keep from the corpus with -Weverything (after an LLVM
# bump or new cuts)
learn: $(WENGINE:%=$(B)/engine/%.o) $(B)/trim/Intrinsics.cpp.obj $(B)/res/wev-base/summary.json
	WFLAGS=-Weverything SC_BASE=$(CURDIR)/$(B)/res/wev-base/summary.json scripts/link.py --learn $(B)/sc-cut.wasm

# what only the native engine calls (cuts/symbols.host.used), after learn
learn-native: $(ENGINE:%=$(B)/engine-host/%.o) $(B)/trim-host/Intrinsics.cpp.o $(B)/res/native-wev-base/summary.json $(SCLOAD)
	WFLAGS=-Weverything SC_BASE=$(CURDIR)/$(B)/res/native-wev-base/summary.json scripts/link.py --native --learn $(B)/link-host-learn/engine

# what only the REPL's session calls (cuts/symbols.session.used), after learn-native
learn-session: $(ENGINE:%=$(B)/engine-host/%.o) $(B)/trim-host/Intrinsics.cpp.o $(B)/res/session-wev-base/summary.json \
    $(B)/res/native-base/summary.json $(SCLOAD)
	scripts/learn_session.py

# after learn-session: what only sc.wasm's session calls, into the same list
learn-session-wasm: $(WENGINE:%=$(B)/engine/%.o) $(B)/trim/Intrinsics.cpp.obj $(B)/res/session-wasm-wev-base/summary.json \
    $(B)/res/native-base/summary.json $(SCLOAD) $(SC_WASM)
	scripts/learn_session.py --wasm

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
	# a session's flags (PLANO-REPL.md §3.5): scIncremental in place of
	# -disable-free and -clear-ast-before-backend
	for a in $(PACK_ARGS:$(B)/pack/repl/%.args=%); do \
	  set -- $$(echo $$a | tr - ' '); l=$$1; shift; \
	  $< $$(scripts/flags.py --session $$l $(CURDIR)/$(B) "$$@" | \
	    sed '/^--$$/,$$d; /^-disable-free$$/d; /^-clear-ast-before-backend$$/d') \
	    -fincremental-extensions -o - /c/x.c || exit 1; \
	done
	$< $$(scripts/flags.py c++ pcm on) -o - /c/std.cppm

size: $(B)/sc.wasm
	@for f in $(wildcard $(B)/sc.wasm $(B)/sc); do printf '%s: %s raw, %s gzip, %s zstd\n' $$f $$(stat -c %s $$f) \
	  $$(gzip -9 -c $$f | wc -c) $$(zstd -19 -c $$f 2>/dev/null | wc -c); done

clean:
	rm -rf $(B)/engine $(B)/engine-host $(B)/link-session $(B)/cargo* $(B)/sc-full $(B)/sc-wasm* $(B)/res/split* $(B)/res/session* $(B)/trim $(B)/trim-host $(B)/link-* $(B)/sc*.wasm $(B)/sc $(B)/res/sc* $(B)/res/native
