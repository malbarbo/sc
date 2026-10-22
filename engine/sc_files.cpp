// The files of sc_set_files (sc.h): for the session's clang, a file system
// that answers for the paths under the root and passes the rest to the real
// one; for its wasm-ld, the hook of llvm/lld-files.patch.
#include "sc.h"
#include "sc_session.h"
#include "llvm/ADT/Hashing.h"
#include "llvm/Support/MemoryBuffer.h"
#include "llvm/Support/Path.h"
#include "llvm/Support/VirtualFileSystem.h"
#include <optional>

namespace lld::wasm {
extern std::optional<llvm::StringRef> (*fileHook)(llvm::StringRef path);
}

using namespace llvm;
namespace fs = llvm::sys::fs;

static const sc_files *Files;

// Path without "." and "..", if it is under the root
static std::optional<std::string> under(const Twine &Path) {
  if (!Files)
    return std::nullopt;
  SmallString<256> P;
  Path.toVector(P);
  sys::path::remove_dots(P, true);
  StringRef R = Files->root, S = P;
  if (!S.consume_front(R) || !(S.empty() || sys::path::is_separator(S[0])))
    return std::nullopt;
  return std::string(P);
}

static std::optional<StringRef> read(const std::string &P) {
  unsigned long long Size;
  if (const char *B = Files->read(P.c_str(), &Size))
    return StringRef(B, Size);
  return std::nullopt;
}

namespace {
vfs::Status makeStatus(const std::string &P, int Kind, uint64_t Size) {
  // the file manager tells files apart by their UniqueID
  return vfs::Status(P, fs::UniqueID(0x5c, hash_value(P)), {}, 0, 0, Size,
                     Kind == 2 ? fs::file_type::directory_file : fs::file_type::regular_file,
                     Kind == 2 ? fs::perms::all_read | fs::perms::all_exe : fs::perms::all_read);
}

struct PackFile : vfs::File {
  vfs::Status S;
  StringRef Bytes;
  PackFile(vfs::Status S, StringRef Bytes) : S(std::move(S)), Bytes(Bytes) {}
  ErrorOr<vfs::Status> status() override { return S; }
  ErrorOr<std::unique_ptr<MemoryBuffer>> getBuffer(const Twine &Name, int64_t, bool RequiresNullTerminator,
                                                    bool) override {
    return MemoryBuffer::getMemBuffer(Bytes, Name.str(), RequiresNullTerminator);
  }
  std::error_code close() override { return {}; }
};

struct PackDir : vfs::detail::DirIterImpl {
  std::string Path;
  StringRef Names; // what is left of them
  explicit PackDir(std::string Path, StringRef Names) : Path(std::move(Path)), Names(Names) { increment(); }
  std::error_code increment() override {
    if (Names.empty()) {
      CurrentEntry = {};
      return {};
    }
    auto [Name, Rest] = Names.split('\0');
    Names = Rest;
    SmallString<256> P(Path);
    sys::path::append(P, Name);
    unsigned long long Size;
    int Kind = Files->status(P.c_str(), &Size);
    CurrentEntry = vfs::directory_entry(std::string(P), Kind == 2 ? fs::file_type::directory_file
                                                                  : fs::file_type::regular_file);
    return {};
  }
};

struct PackFS : vfs::ProxyFileSystem {
  using ProxyFileSystem::ProxyFileSystem;
  ErrorOr<vfs::Status> status(const Twine &Path) override {
    std::optional<std::string> P = under(Path);
    if (!P)
      return ProxyFileSystem::status(Path);
    unsigned long long Size = 0;
    if (int Kind = Files->status(P->c_str(), &Size))
      return makeStatus(*P, Kind, Size);
    return std::make_error_code(std::errc::no_such_file_or_directory);
  }
  bool exists(const Twine &Path) override {
    if (!under(Path))
      return ProxyFileSystem::exists(Path);
    return bool(status(Path));
  }
  ErrorOr<std::unique_ptr<vfs::File>> openFileForRead(const Twine &Path) override {
    std::optional<std::string> P = under(Path);
    if (!P)
      return ProxyFileSystem::openFileForRead(Path);
    if (std::optional<StringRef> B = read(*P))
      return std::make_unique<PackFile>(makeStatus(*P, 1, B->size()), *B);
    return std::make_error_code(std::errc::no_such_file_or_directory);
  }
  vfs::directory_iterator dir_begin(const Twine &Path, std::error_code &EC) override {
    std::optional<std::string> P = under(Path);
    if (!P)
      return ProxyFileSystem::dir_begin(Path, EC);
    unsigned long long Size;
    const char *Names = Files->list(P->c_str(), &Size);
    if (!Names) {
      EC = std::make_error_code(std::errc::not_a_directory);
      return {};
    }
    return vfs::directory_iterator(std::make_shared<PackDir>(*P, StringRef(Names, Size)));
  }
  std::error_code getRealPath(const Twine &Path, SmallVectorImpl<char> &Output) override {
    std::optional<std::string> P = under(Path);
    if (!P)
      return ProxyFileSystem::getRealPath(Path, Output);
    Output.assign(P->begin(), P->end());
    return {};
  }
  std::error_code isLocal(const Twine &Path, bool &Result) override {
    if (!under(Path))
      return ProxyFileSystem::isLocal(Path, Result);
    Result = true;
    return {};
  }
};
} // namespace

void sc_set_files(const sc_files *F) {
  Files = F;
  lld::wasm::fileHook = F ? +[](StringRef Path) -> std::optional<StringRef> {
    std::optional<std::string> P = under(Path);
    return P ? read(*P) : std::nullopt;
  } : nullptr;
}

IntrusiveRefCntPtr<vfs::FileSystem> scFileSystem() {
  IntrusiveRefCntPtr<vfs::FileSystem> Real = vfs::getRealFileSystem();
  return Files ? makeIntrusiveRefCnt<PackFS>(Real) : Real;
}
