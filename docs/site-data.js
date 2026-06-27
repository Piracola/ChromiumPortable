window.SITE_DATA = {
  meta: {
    title: "Chromium Portable Builder",
    description:
      "Chrome、Edge、Helium 便携构建版的统一索引。按浏览器系列查看目标、频道、仓库和最新发布。",
    builderRepo: "https://github.com/Piracola/ChromiumPortable",
    ctaLink: "https://github.com/Piracola/ChromiumPortable"
  },
  repos: [
    {
      id: "chrome",
      name: "Chrome-Portable",
      owner: "Piracola",
      repo: "Chrome-Portable",
      url: "https://github.com/Piracola/Chrome-Portable",
      releasesUrl: "https://github.com/Piracola/Chrome-Portable/releases/latest",
      workflowUrl: "https://github.com/Piracola/Chrome-Portable/actions/workflows/build.yml",
      icon: "./assets/chrome.svg",
      accent: "#d8653f",
      badge: "CH",
      summary:
        "自动构建的 Chrome 便携版项目，当前同时维护 Stable 与 Beta 两个频道。"
    },
    {
      id: "edge",
      name: "Edge_Portable",
      owner: "betacola",
      repo: "Edge_Portable",
      url: "https://github.com/betacola/Edge_Portable",
      releasesUrl: "https://github.com/betacola/Edge_Portable/releases/latest",
      workflowUrl: "https://github.com/betacola/Edge_Portable/actions/workflows/build.yml",
      icon: "./assets/edge.svg",
      accent: "#1d7c84",
      badge: "ED",
      summary:
        "自动构建的 Microsoft Edge 便携版项目，当前维护 Stable 渠道。"
    },
    {
      id: "helium",
      name: "Helium_Portable",
      owner: "Piracola",
      repo: "Helium_Portable",
      url: "https://github.com/Piracola/Helium_Portable",
      releasesUrl: "https://github.com/Piracola/Helium_Portable/releases/latest",
      workflowUrl: "https://github.com/Piracola/Helium_Portable/actions/workflows/build.yml",
      icon: "./assets/helium.svg",
      accent: "#5b5bd6",
      badge: "HE",
      summary:
        "自动构建的 Helium Windows x64 便携版项目，同时展示 Stable 与 Preview。"
    }
  ],
  builds: [
    {
      id: "chrome-stable",
      repoId: "chrome",
      project: "Chrome",
      title: "Chrome++",
      channel: "Stable",
      family: "Chrome",
      summary:
        "面向日常使用的稳定构建，集成 Chrome++ 便携化增强组件。",
      architecture: "x64",
      outputDir: "Chrome",
      target: "chrome_stable",
      highlight: "主力版本",
      color: "#d8653f",
      icon: "./assets/chrome.svg",
      links: {
        repo: "https://github.com/Piracola/Chrome-Portable",
        releases: "https://github.com/Piracola/Chrome-Portable/releases/latest",
        workflow: "https://github.com/Piracola/Chrome-Portable/actions/workflows/build.yml"
      }
    },
    {
      id: "chrome-beta",
      repoId: "chrome",
      project: "Chrome",
      title: "Chrome++ Beta",
      channel: "Beta",
      family: "Chrome",
      summary:
        "用于提前体验 Chrome 新版本能力的测试频道，不建议和 Stable 频繁混用数据目录。",
      architecture: "x64",
      outputDir: "Chrome",
      target: "chrome_beta",
      highlight: "抢先体验",
      color: "#e08a48",
      icon: "./assets/chrome.svg",
      links: {
        repo: "https://github.com/Piracola/Chrome-Portable",
        releases: "https://github.com/Piracola/Chrome-Portable/releases/latest",
        workflow: "https://github.com/Piracola/Chrome-Portable/actions/workflows/build.yml"
      }
    },
    {
      id: "edge-stable",
      repoId: "edge",
      project: "Edge",
      title: "Microsoft Edge",
      channel: "Stable",
      family: "Edge",
      summary:
        "Microsoft Edge 便携稳定版，适合希望保留 Edge 生态兼容性的用户。",
      architecture: "x64",
      outputDir: "Edge",
      target: "edge_stable",
      highlight: "微软生态",
      color: "#1d7c84",
      icon: "./assets/edge.svg",
      links: {
        repo: "https://github.com/betacola/Edge_Portable",
        releases: "https://github.com/betacola/Edge_Portable/releases/latest",
        workflow: "https://github.com/betacola/Edge_Portable/actions/workflows/build.yml"
      }
    },
    {
      id: "helium-stable",
      repoId: "helium",
      project: "Helium",
      title: "Helium Portable",
      channel: "Stable",
      family: "Helium",
      summary:
        "Helium 正式版便携包，适合偏好轻量化 Chromium 分支体验的用户。",
      architecture: "x64",
      outputDir: "Helium",
      target: "helium_stable",
      highlight: "轻量分支",
      color: "#5b5bd6",
      icon: "./assets/helium.svg",
      links: {
        repo: "https://github.com/Piracola/Helium_Portable",
        releases: "https://github.com/Piracola/Helium_Portable/releases/latest",
        workflow: "https://github.com/Piracola/Helium_Portable/actions/workflows/build.yml"
      }
    },
    {
      id: "helium-preview",
      repoId: "helium",
      project: "Helium",
      title: "Helium Portable Preview",
      channel: "Preview",
      family: "Helium",
      summary:
        "Helium 预发行便携包，方便跟进上游新特性与预览更新。",
      architecture: "x64",
      outputDir: "Helium",
      target: "helium_prerelease",
      highlight: "预发行",
      color: "#7b6ff0",
      icon: "./assets/helium.svg",
      links: {
        repo: "https://github.com/Piracola/Helium_Portable",
        releases: "https://github.com/Piracola/Helium_Portable/releases/latest",
        workflow: "https://github.com/Piracola/Helium_Portable/actions/workflows/build.yml"
      }
    }
  ]
};
