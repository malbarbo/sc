// The main of the engine's executables (sc.wasm, build/sc...). A host that
// links the engine itself calls sc_main instead (sc.h).
#include "sc.h"

int main(int argc, const char **argv) { return sc_main(argc, argv); }
