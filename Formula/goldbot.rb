class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.8.7"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.7/goldbot-v0.8.7-macos-aarch64.tar.gz"
      sha256 "929e12f241e848d4d7f06af847c3ff23be11fdf7efb1be0e847598e0177d1cbe"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.7/goldbot-v0.8.7-macos-x86_64.tar.gz"
      sha256 "ed906165dc107b8f9b61e2fe073957a1a53e6dae0c3b83f7f8cf68c8d90ea70a"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.7/goldbot-v0.8.7-linux-x86_64.tar.gz"
    sha256 "bf4f7953ddaff2d5f303383f3d0b56c5685e9886b31fb75fa839d139b5de2057"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
