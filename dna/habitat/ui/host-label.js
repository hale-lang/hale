// The habitat's own machine is the host `local` in its rows: the one no SSH
// reaches, which the habitat's code branches on. A person knows it by the
// name they reached this page at (`keep` for a habitat started with
// `hale habitat up --host keep`), so that is what the screens show; the
// value sent back to the head stays `local`.
export function hostLabel(name) {
  if (name !== "local") return name;
  const h = location.hostname;
  const loopback = h === "" || h === "127.0.0.1" || h === "localhost" || h === "::1" || h === "[::1]";
  return loopback ? "local" : h;
}
