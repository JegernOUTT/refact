class Refact < Formula
  desc "Open-source, local-first agentic coding engine"
  homepage "https://github.com/JegernOUTT/refact"
  version "__REFACT_VERSION__"
  license "BSD-3-Clause"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/JegernOUTT/refact/releases/download/engine/v#{version}/refact-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "__REFACT_SHA256_AARCH64_APPLE_DARWIN__"
    else
      url "https://github.com/JegernOUTT/refact/releases/download/engine/v#{version}/refact-#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "__REFACT_SHA256_X86_64_APPLE_DARWIN__"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/JegernOUTT/refact/releases/download/engine/v#{version}/refact-#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "__REFACT_SHA256_AARCH64_UNKNOWN_LINUX_GNU__"
    else
      url "https://github.com/JegernOUTT/refact/releases/download/engine/v#{version}/refact-#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "__REFACT_SHA256_X86_64_UNKNOWN_LINUX_GNU__"
    end
  end

  def install
    bin.install "refact"
  end

  test do
    system "#{bin}/refact", "version"
  end
end
