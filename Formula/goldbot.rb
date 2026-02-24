class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.7.8"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.8/goldbot-v0.7.8-macos-aarch64.tar.gz"
      sha256 "9d9c1fdc4b973c501b12417aa668d323cf90871c4a6987963fa6208d9a8032bb"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.8/goldbot-v0.7.8-macos-x86_64.tar.gz"
      sha256 "8236bc2b7d1d41f6def7f1975b696fc5de2f0b61e0301416401bae632a9181cc"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.8/goldbot-v0.7.8-linux-x86_64.tar.gz"
    sha256 "2a13b88d56ecd11759c3b2b90c59274c92b26fd2a78069fba9eab8c0a66f1411"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
