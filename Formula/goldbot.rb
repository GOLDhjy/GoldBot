class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.7.10"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.10/goldbot-v0.7.10-macos-aarch64.tar.gz"
      sha256 "65e8c491d3081681765cf4412830fb57b137d0f891b4923f19328d59abab6d8c"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.10/goldbot-v0.7.10-macos-x86_64.tar.gz"
      sha256 "9c0c2a0f0910a765abd4395bf1e5c98fa2c6a1204dd8a9c817ddb566513d7e57"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.10/goldbot-v0.7.10-linux-x86_64.tar.gz"
    sha256 "57b10d1b127916a6a319c61b830847d11ee10f00841d2101d628fa7ebfa4817c"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
