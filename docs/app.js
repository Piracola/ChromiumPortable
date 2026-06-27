(function () {
  const data = window.SITE_DATA;
  const repoMap = new Map(data.repos.map((repo) => [repo.id, repo]));
  const buildGrid = document.getElementById("buildGrid");
  const repoGrid = document.getElementById("repoGrid");
  const filterBar = document.getElementById("filterBar");

  const state = {
    filter: "all"
  };

  const families = ["all", ...new Set(data.builds.map((build) => build.family))];

  function setMeta() {
    document.title = data.meta.title;
    document.getElementById("heroTitle").textContent = data.meta.title;
    document.getElementById("heroDescription").textContent = data.meta.description;
    document.getElementById("builderRepoLink").href = data.meta.builderRepo;
    document.getElementById("allReleasesLink").href = data.meta.ctaLink;
  }

  function renderFilters() {
    filterBar.innerHTML = families
      .map((family) => {
        const active = state.filter === family ? " is-active" : "";
        const label = family === "all" ? "全部" : family;
        return `<button class="filter-chip${active}" type="button" data-filter="${family}">${label}</button>`;
      })
      .join("");

    filterBar.querySelectorAll("[data-filter]").forEach((button) => {
      button.addEventListener("click", () => {
        state.filter = button.dataset.filter;
        renderFilters();
        renderBuilds();
      });
    });
  }

  function buildCard(build, index) {
    const repo = repoMap.get(build.repoId);
    const visible = state.filter === "all" || state.filter === build.family;
    const hiddenClass = visible ? "" : " is-hidden";
    const delay = 80 + index * 60;

    return `
      <article class="build-card reveal${hiddenClass}" style="--accent:${build.color}; animation-delay:${delay}ms">
        <div class="build-head">
          <img class="browser-icon" src="${build.icon}" alt="${build.project} 图标" loading="lazy" />
          <div class="build-title">
            <h3>${build.title}</h3>
            <span class="build-subtitle">${build.project} / ${build.channel} / ${build.highlight}</span>
          </div>
          <span class="channel-pill">${build.channel}</span>
        </div>

        <p class="build-summary">${build.summary}</p>

        <dl class="build-meta">
          <div class="metric">
            <dt>Target</dt>
            <dd>${build.target}</dd>
          </div>
          <div class="metric">
            <dt>Output</dt>
            <dd>${build.outputDir}</dd>
          </div>
          <div class="metric">
            <dt>Architecture</dt>
            <dd>${build.architecture}</dd>
          </div>
          <div class="metric">
            <dt>Project</dt>
            <dd>${repo.name}</dd>
          </div>
        </dl>

        <div class="build-links">
          <a class="link-chip primary" href="${build.links.repo}" target="_blank" rel="noreferrer">进入项目</a>
          <a class="link-chip" href="${build.links.releases}" target="_blank" rel="noreferrer">Latest Release</a>
          <a class="link-chip" href="${build.links.workflow}" target="_blank" rel="noreferrer">Workflow</a>
        </div>
      </article>
    `;
  }

  function renderBuilds() {
    buildGrid.innerHTML = data.builds.map(buildCard).join("");
  }

  function repoCard(repo, index) {
    const repoBuilds = data.builds.filter((build) => build.repoId === repo.id);
    const channels = [...new Set(repoBuilds.map((build) => build.channel))].join(" / ");
    const delay = 120 + index * 70;

    return `
      <article class="repo-card reveal" style="animation-delay:${delay}ms">
        <div class="repo-head">
          <img class="repo-icon" src="${repo.icon}" alt="${repo.name} 图标" loading="lazy" />
          <div class="repo-copy">
            <h3>${repo.name}</h3>
            <span class="repo-meta">${repo.owner} · ${repoBuilds.length} builds</span>
          </div>
        </div>
        <p>${repo.summary}</p>
        <p class="repo-detail"><span>Channels</span> ${channels}</p>
        <div class="repo-links">
          <a class="link-chip primary" href="${repo.url}" target="_blank" rel="noreferrer">仓库主页</a>
          <a class="link-chip" href="${repo.releasesUrl}" target="_blank" rel="noreferrer">Releases</a>
          <a class="link-chip" href="${repo.workflowUrl}" target="_blank" rel="noreferrer">Actions</a>
        </div>
      </article>
    `;
  }

  function renderRepos() {
    repoGrid.innerHTML = data.repos.map(repoCard).join("");
  }

  setMeta();
  renderFilters();
  renderBuilds();
  renderRepos();
})();
