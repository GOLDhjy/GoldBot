class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.6.7"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.6.7/goldbot-v0.6.7-macos-aarch64.tar.gz"
      sha256 "3fde94e91fc1132faf79506f43c97a1d69547bf08047b91ae4467b41a30b809a"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.6.7/goldbot-v0.6.7-macos-x86_64.tar.gz"
      sha256 "4ab7a18c7a942199ff2b6c6ee245e7ca672ae51f95889c18b4f9c99adb55f624"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.6.7/goldbot-v0.6.7-linux-x86_64.tar.gz"
    sha256 "f46ef6bdeb5914b998fbb2b81208d23c0678b3a842444aae20e1bd8e990329ea"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
