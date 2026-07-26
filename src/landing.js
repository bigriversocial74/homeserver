import "./landing.css";
import { icon, logoMark } from "./icons.js";

const features = [
  ["model", "Models", "Run local AI models with full control."],
  ["vault", "Knowledge Vault", "Index and search your private knowledge."],
  ["cloud", "Sync & Cloud", "Secure sync and relay when you need it."],
  ["integrations", "Integrations", "Connect tools and automate workflows."],
  ["agent", "Agents", "Run local agents that work for you."],
  ["backup", "Backups", "Automated backups and verified recovery."],
  ["settings", "Settings", "Configure, secure, and stay in control."],
];

const heroAsset = new URL("../homeserver-index-page (1).png", import.meta.url).href;

const app = document.querySelector("#landing-app");
app.innerHTML = `
  <div class="landing-shell">
    <header class="landing-header">
      <a class="landing-brand" href="#top" aria-label="Microgifter HomeServer home">${logoMark(46)}<span><strong>MICROGIFTER</strong><em>HomeServer</em></span></a>
      <nav aria-label="Primary navigation"><a href="#product">Product</a><a href="#solutions">Solutions</a><a href="#resources">Resources</a><a href="#pricing">Pricing</a><a href="#docs">Docs</a></nav>
      <div class="header-actions"><a class="landing-button ghost" href="https://microgifter.com/login.php">Sign in</a><a class="landing-button primary" href="#download">Get HomeServer</a></div>
    </header>

    <section class="landing-hero" id="top">
      <div class="hero-copy">
        <span class="hero-kicker"><i></i>LOCAL-FIRST. PRIVATE BY DESIGN.</span>
        <h1>Private AI.<br>Local Control.<br>Cloud Connected.</h1>
        <p>HomeServer is your private Microgifter node for running local AI models, managing knowledge, integrating tools, and staying secure—on your terms.</p>
        <div class="hero-actions"><a class="landing-button primary large" href="#download">Get HomeServer ${icon("arrow", 18)}</a><a class="landing-button ghost large" href="#product">Explore Features ${icon("arrow", 18)}</a></div>
        <div class="trust-row"><span>${icon("lock", 18)}Private by default</span><span>${icon("shield", 18)}Your data stays yours</span><span>${icon("cloud", 18)}Optional cloud services</span></div>
      </div>
      <div class="hero-visual" aria-label="Microgifter HomeServer Control Center and compact HomeServer device"><div class="hero-glow"></div><div class="hero-crop"><img src="${heroAsset}" alt="Microgifter HomeServer Control Center displayed beside a compact HomeServer device"></div></div>
    </section>

    <section class="feature-strip" id="product">
      ${features.map(([iconName, title, copy]) => `<article>${icon(iconName, 24)}<h2>${title}</h2><p>${copy}</p></article>`).join("")}
    </section>

    <section class="product-story" id="solutions">
      <div><span class="section-label">ONE PRIVATE NODE</span><h2>Everything important stays close.</h2><p>HomeServer combines a Windows background service, local API, encrypted backups, signed cloud pairing, and a polished Control Center into one private operating layer for Microgifter.</p></div>
      <div class="story-grid"><article>${icon("shield", 25)}<h3>Local authority</h3><p>Keys, models, files, integrations, automations, backups, and diagnostics stay under local control.</p></article><article>${icon("cloud", 25)}<h3>Cloud connection</h3><p>Pair securely with Microgifter using signed requests, scoped permissions, replay protection, and durable receipts.</p></article><article>${icon("backup", 25)}<h3>Verified recovery</h3><p>Create encrypted local backups and portable recovery packages with explicit verification and staged restore.</p></article></div>
    </section>

    <section class="download-panel" id="download"><div><span class="section-label">HOME SERVER v0.1.3</span><h2>Bring Microgifter home.</h2><p>Install the private Control Center, pair your account, and begin testing local sync, backups, and service health.</p></div><div><a class="landing-button primary large" href="https://github.com/bigriversocial74/homeserver/releases">Download HomeServer ${icon("download", 18)}</a><a class="landing-button ghost large" href="https://microgifter.com/account-homeserver.php">Pair Your Device ${icon("external", 17)}</a></div></section>

    <footer class="landing-footer"><div class="landing-brand">${logoMark(38)}<span><strong>MICROGIFTER</strong><em>HomeServer</em></span></div><p>Private local infrastructure for the Microgifter ecosystem.</p><span>© ${new Date().getFullYear()} Microgifter</span></footer>
  </div>`;
