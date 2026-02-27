class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.8.8"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.8/goldbot-v0.8.8-macos-aarch64.tar.gz"
      sha256 "e12b944dfe2431d39806dd11eae5d303640b8ed4e9a785a6e7aa3906d10671c3"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.8/goldbot-v0.8.8-macos-x86_64.tar.gz"
      sha256 "c8b49794232cf664fdf3625b8288d5935189e14e24cb8565e737212f8837f1fa"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.8/goldbot-v0.8.8-linux-x86_64.tar.gz"
    sha256 "e536194d3ebc1116767cb7cc781952e70cffdd30329915299920c5edb9a5c715"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
