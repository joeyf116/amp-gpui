// Fills in the Download page from GitHub's list of releases. The page's static
// links still work if this can't load, such as when the API rate limit is hit.
(() => {
  const ASSET = "amp-gpui-setup.exe";

  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text) node.textContent = text;
    return node;
  }

  function link(href, text, className) {
    const node = element("a", className, text);
    node.href = href;
    return node;
  }

  function formatDate(iso) {
    return new Date(iso).toLocaleDateString(undefined, { dateStyle: "long" });
  }

  function formatSize(bytes) {
    return `${(bytes / 1024 / 1024).toFixed(0)} MB`;
  }

  function installer(release) {
    return release.assets.find((asset) => asset.name === ASSET);
  }

  function renderLatest(release) {
    const asset = installer(release);
    const card = element("div", "dl-latest");
    card.append(
      element("p", "dl-label", "Latest version"),
      element("p", "dl-version", release.name || release.tag_name),
      link(asset.browser_download_url, "Download for Windows", "md-button md-button--primary"),
      element("p", "dl-meta", `${formatDate(release.published_at)} · ${formatSize(asset.size)}`),
      link(release.html_url, "What's new"),
    );
    return card;
  }

  function renderRow(release) {
    const version = element("td");
    version.append(link(release.html_url, release.tag_name));
    if (release.prerelease) version.append(element("span", "dl-tag", "Prerelease"));
    const download = element("td");
    download.append(link(installer(release).browser_download_url, "Download"));
    const row = element("tr");
    row.append(version, element("td", "", formatDate(release.published_at)), download);
    return row;
  }

  function renderTable(releases) {
    const head = element("tr");
    for (const title of ["Version", "Released", "Installer"]) {
      head.append(element("th", "", title));
    }
    const thead = element("thead");
    thead.append(head);
    const tbody = element("tbody");
    tbody.append(...releases.map(renderRow));
    const table = element("table");
    table.append(thead, tbody);
    return table;
  }

  function render(root, repo, releases) {
    const usable = releases.filter(installer);
    if (!usable.length) {
      root.replaceChildren(element("p", "", "The first release is on its way."));
      return;
    }
    const latest = usable.find((release) => !release.prerelease);
    const others = usable.filter((release) => release !== latest);
    const nodes = [];
    if (latest) nodes.push(renderLatest(latest));
    if (others.length) {
      nodes.push(element("h2", "", latest ? "Other versions" : "Prereleases"), renderTable(others));
    }
    nodes.push(link(`https://github.com/${repo}/releases`, "All releases on GitHub"));
    root.replaceChildren(...nodes);
  }

  async function start() {
    const root = document.getElementById("releases");
    if (!root || root.dataset.ready) return;
    root.dataset.ready = "true";
    const repo = root.dataset.repo;
    const response = await fetch(`https://api.github.com/repos/${repo}/releases?per_page=30`);
    if (!response.ok) return;
    render(root, repo, await response.json());
  }

  // The site swaps pages without a reload, so this runs on each one.
  const run = () => start().catch(() => {});
  if (globalThis.document$) document$.subscribe(run);
  else run();
})();
