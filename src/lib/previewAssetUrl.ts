export function previewAssetUrl(source: string, parameter: string, revision: string) {
  // Imported Gaussian projects use a virtual PLY identity rather than an asset:// URL.
  // Resolve that identity against the current app origin so URL search parameters can still
  // be used to invalidate PlayCanvas' asset cache without ever fetching the virtual URL.
  const base = globalThis.location?.href ?? "http://ooosplat.local/";
  const url = new URL(source, base);
  url.searchParams.set(parameter, revision);
  return url.toString();
}
