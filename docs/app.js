(function () {
  const data = window.SITE_DATA;
  const repoMap = new Map(data.repos.map((repo) => [repo.id, repo]));
  const buildGrid = document.getElementById("buildGrid");
  const repoGrid = document.getElementById("repoGrid");
  const filterBar = document.getElementById("filterBar");
  const heroStats = document.getElementById("heroStats");

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
    document.getElementById("buildCountBadge").textContent = `${data.builds.length} builds`;
  }

  function statCard(label, value, copy) {
    return `
      <article class="stat-card reveal">
        <span class="stat-label">${label}</span>
        <div class="stat-value">
          <strong>${value}</strong>
          <span>${copy}</span>
        </div>
      </article>
    `;
  }

  function renderStats() {
    const channels = new Set(data.builds.map((build) => build.channel));
    const repos = data.repos.length;
    const architectures = new Set(data.builds.map((build) => build.architecture));

    heroStats.innerHTML = [
      statCard("构建目标", String(data.builds.length), "个子版本入口"),
      statCard("子项目", String(repos), "个独立仓库"),
      statCard("频道类型", String(channels.size), "种更新分支"),
      statCard("架构", String(architectures.size), "种输出规格")
    ].join("");
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
    const gradient = `linear-gradient(135deg, ${build.color}22, ${build.color}08)`;
    const pillStyle = `background:${build.color}18;color:${build.color};`;

    return `
      <article class="build-card reveal${hiddenClass}" style="animation-delay:${delay}ms;background:${gradient}">
        <div class="build-head">
          <div class="build-title">
            <h3>${build.title}</h3>
            <span class="build-subtitle">${build.project} / ${build.channel} / ${build.highlight}</span>
          </div>
          <span class="channel-pill" style="${pillStyle}">${build.channel}</span>
        </div>

        <p class="build-summary">${build.summary}</p>

        <div class="build-metrics">
          <article class="metric">
            <span class="metric-label">Target</span>
            <div class="metric-copy">${build.target}</div>
          </article>
          <article class="metric">
            <span class="metric-label">Output</span>
            <div class="metric-copy">${build.outputDir}</div>
          </article>
          <article class="metric">
            <span class="metric-label">Architecture</span>
            <div class="metric-copy">${build.architecture}</div>
          </article>
          <article class="metric">
            <span class="metric-label">Project</span>
            <div class="metric-copy">${repo.name}</div>
          </article>
        </div>

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
          <span class="repo-mark" style="background:linear-gradient(135deg, ${repo.accent}, #13212f)">${repo.badge}</span>
          <div class="repo-copy">
            <h3>${repo.name}</h3>
            <span class="repo-meta">${repo.owner} · ${repoBuilds.length} builds</span>
          </div>
        </div>
        <p>${repo.summary}</p>
        <div class="metric">
          <span class="metric-label">Channels</span>
          <div class="metric-copy">${channels}</div>
        </div>
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
  renderStats();
  renderFilters();
  renderBuilds();
  renderRepos();
})();
