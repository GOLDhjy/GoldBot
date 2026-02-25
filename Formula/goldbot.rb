class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.7.13"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.13/goldbot-v0.7.13-macos-aarch64.tar.gz"
      sha256 "49775bfede4460b60469ebc082c36a4e184d1ada8a3ccffc6a9b066947c64a54"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.13/goldbot-v0.7.13-macos-x86_64.tar.gz"
      sha256 "91f640f96a3383d089963c450198b1c442188cabfa00d545c3b070b203b8d3ff"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.13/goldbot-v0.7.13-linux-x86_64.tar.gz"
    sha256 "b53f92fe664231f468bcdf74c1e0446e53d07a086ca1061e1dbb50861ebda608"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
