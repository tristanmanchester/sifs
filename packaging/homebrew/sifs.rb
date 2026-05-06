class Sifs < Formula
  desc "SIFS Is Fast Search: instant local code search for agents"
  homepage "https://github.com/tristanmanchester/sifs"
  url "https://crates.io/api/v1/crates/sifs/0.3.2/download"
  sha256 "d0dc5d6cad1c5f2dcb0e2650165fdb719208e7da1903d6d27ee79911f1d6cc32"
  version "0.3.2"
  license "MIT"
  head "https://github.com/tristanmanchester/sifs.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", "--path", ".", "--root", prefix
  end

  test do
    repo = testpath/"tiny-repo"
    repo.mkpath
    (repo/"auth.py").write <<~PY
      def authenticate_token(token):
          return token == "secret"
    PY

    system "git", "-C", repo, "init", "--quiet"

    output = shell_output("#{bin}/sifs search authenticate_token --source #{repo} --mode bm25 --offline --no-cache")
    assert_match "auth.py", output
    assert_match "authenticate_token", output
  end
end
