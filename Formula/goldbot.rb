class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.8.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.0/goldbot-v0.8.0-macos-aarch64.tar.gz"
      sha256 "c251e12c5fb08a1aa67983515a17482258d6a7c81db779cef32ed2f944d21fee"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.0/goldbot-v0.8.0-macos-x86_64.tar.gz"
      sha256 "570a4f3345b626ea7da481351f7ab6017b8af23384fc0949d0230edd76090799"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.0/goldbot-v0.8.0-linux-x86_64.tar.gz"
    sha256 "5ef3aabfb5ddec73fbe27e3ac980f2906f13e6119ee45933a8a68cf40b3fdbf0"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
