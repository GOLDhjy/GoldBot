class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.9.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.0/goldbot-v0.9.0-macos-aarch64.tar.gz"
      sha256 "fdc7fba92208a1d2c8df37877a884adb27c8fe29aeefe3ddc0bc27f72fddfca3"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.0/goldbot-v0.9.0-macos-x86_64.tar.gz"
      sha256 "11cb018a9bf20fc5be9506627b8ce754b104a3920f41bb1aa6e1c3f2aa7d9823"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.0/goldbot-v0.9.0-linux-x86_64.tar.gz"
    sha256 "4643868cd441cedf5d7f298ecfbfbb12ea8ca6e30b19147b97c55560ed0d993c"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
