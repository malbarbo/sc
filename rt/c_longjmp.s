# The tag of setjmp/longjmp, defined strong for libsetjmp.so. wasi-sdk's
# libsetjmp.a defines it weak, and wasi-sdk's libsetjmp.so imports it: a
# program linked with --unresolved-symbols=report-all (sc FILE) does not
# find it, and a weak definition that wasm-ld exports (dylink-weak.patch)
# throws a tag of its own, not the one the loader gives the rest
	.tagtype	__c_longjmp i32
	.globl	__c_longjmp
__c_longjmp:
