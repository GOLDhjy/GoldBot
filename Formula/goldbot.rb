class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.9.23"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.23/goldbot-v0.9.23-macos-aarch64.tar.gz"
      sha256 "f6e0933501156617a9a809ff22ca86bf44d946cfc5957d61c46fa37871af4240"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.23/goldbot-v0.9.23-macos-x86_64.tar.gz"
      sha256 "c79b4ea89ead6f46a74d25b4642c7a8ed457f94c6e1bb74b1dda12664b90338b"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.23/goldbot-v0.9.23-linux-x86_64.tar.gz"
    sha256 "4f9d9fb41227eb58cd426bce1bd0f726165424111ca2fc4b2b2d6d24b9c7d101"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
