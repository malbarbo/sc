#pragma once
#include "llvm/ADT/ArrayRef.h"
#include <string>
#include <vector>

namespace clang { class CompilerInvocation; }

struct ScOptions {
  std::string Input, Output = "-", Sysroot, ResourceDir;
  std::vector<std::string> ISystem, ExternCISystem;
  bool CXX = false;
};

bool scParseArgs(llvm::ArrayRef<const char *> Args, ScOptions &O);
void scBuildInvocation(clang::CompilerInvocation &Inv, const ScOptions &O);
