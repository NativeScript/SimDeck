<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";

const command = "npm i -g simdeck@latest\nsimdeck";
const copied = ref(false);

function updateNavState() {
  const scrollY = window.scrollY;
  const progress = Math.min(Math.max((scrollY - 120) / 260, 0), 1);
  const bgCameraY = Math.min(scrollY * 0.58, window.innerHeight * 0.7);
  const bgOpacity = Math.max(1 - scrollY / (window.innerHeight * 0.95), 0);

  document.documentElement.style.setProperty(
    "--sd-nav-progress",
    progress.toFixed(3),
  );
  document.documentElement.style.setProperty(
    "--sd-hero-bg-camera-y",
    `${(-bgCameraY).toFixed(1)}px`,
  );
  document.documentElement.style.setProperty(
    "--sd-hero-bg-opacity",
    bgOpacity.toFixed(3),
  );
  document.documentElement.classList.toggle("sd-nav-scrolled", progress > 0.02);
}

onMounted(() => {
  document.documentElement.classList.add("sd-home-active");
  updateNavState();
  window.addEventListener("scroll", updateNavState, { passive: true });
});

onBeforeUnmount(() => {
  window.removeEventListener("scroll", updateNavState);
  document.documentElement.classList.remove(
    "sd-home-active",
    "sd-nav-scrolled",
  );
  document.documentElement.style.removeProperty("--sd-nav-progress");
  document.documentElement.style.removeProperty("--sd-hero-bg-camera-y");
  document.documentElement.style.removeProperty("--sd-hero-bg-opacity");
});

async function copyCommand() {
  try {
    const textarea = document.createElement("textarea");
    textarea.value = command;
    textarea.setAttribute("readonly", "");
    textarea.style.position = "fixed";
    textarea.style.opacity = "0";
    document.body.appendChild(textarea);
    textarea.select();
    const copiedViaSelection = document.execCommand("copy");
    textarea.remove();
    if (
      !copiedViaSelection &&
      typeof navigator !== "undefined" &&
      navigator.clipboard
    ) {
      await navigator.clipboard.writeText(command);
    }
  } catch {
    return;
  }
  copied.value = true;
  window.setTimeout(() => {
    copied.value = false;
  }, 1500);
}
</script>

<template>
  <main class="sd-home">
    <section class="sd-hero" aria-labelledby="simdeck-title">
      <div class="sd-hero-copy sd-reveal">
        <h1 id="simdeck-title">SimDeck</h1>
        <p>Simulator Superpowers for you and your fleet of agents</p>
        <div class="sd-command-wrap" aria-label="Install and start SimDeck">
          <pre
            class="sd-command"
          ><code><span>npm i -g simdeck@latest</span><span>simdeck</span></code></pre>
          <button
            class="sd-copy"
            type="button"
            :aria-label="
              copied
                ? 'Copied SimDeck install command'
                : 'Copy SimDeck install command'
            "
            :data-copied="copied ? 'true' : 'false'"
            @click="copyCommand"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path v-if="copied" d="m5 12 4 4 10-10" />
              <template v-else>
                <path d="M9 9h10v10H9z" />
                <path
                  d="M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1"
                />
              </template>
            </svg>
          </button>
        </div>
      </div>

      <figure class="sd-hero-shot sd-reveal">
        <img
          src="/images/codex-screenshot.png"
          alt="SimDeck streaming an iPhone simulator inside a development workspace"
        />
      </figure>
    </section>

    <section class="sd-story" aria-labelledby="simdeck-story">
      <div class="sd-story-head sd-reveal">
        <h2 id="simdeck-story">
          SimDeck does not get in the way. It makes the way for agents to work
          without dividing your attention.
        </h2>
      </div>

      <div class="sd-timeline">
        <article class="sd-moment sd-reveal">
          <div class="sd-index" aria-hidden="true">01</div>
          <div class="sd-moment-copy">
            <h3>Live Simulator, where your work already happens</h3>
            <p>
              Open Simulator side by side in your favorite IDE or agent
              orchestrator. Supported in VS Code, Codex, Cursor, Claude Code,
              and more. Simulators run headlessly in the background for agents
              to operate on while you stay focused on what matters in the
              current context.
            </p>
          </div>
          <figure class="sd-shot sd-shot-image sd-shot-ide">
            <img
              src="/images/vscode-screenshot.png"
              alt="SimDeck running beside code in VS Code"
            />
          </figure>
        </article>

        <article class="sd-moment sd-reveal">
          <div class="sd-index" aria-hidden="true">02</div>
          <div class="sd-moment-copy">
            <h3>Pull request previews for mobile apps</h3>
            <p>
              GitHub Actions can build the app, boot a simulator, and stream the
              session back through SimDeck. Review a mobile PR like a web
              preview: open it, tap around, and merge with actual confidence.
            </p>
          </div>
          <figure class="sd-shot sd-shot-image sd-shot-pr-image">
            <img
              src="/images/pr-comment.png"
              alt="SimDeck pull request preview comment with a streamed simulator session"
            />
          </figure>
        </article>

        <article class="sd-moment sd-reveal">
          <div class="sd-index" aria-hidden="true">03</div>
          <div class="sd-moment-copy">
            <h3>Remote simulator access without the screen-share lag</h3>
            <p>
              WebRTC carries the live stream to browsers, teammates, and SimDeck
              Studio on iOS. Pair once, then drive the simulator from wherever
              the work is happening.
            </p>
          </div>
          <figure class="sd-shot sd-shot-remote-card">
            <img
              src="/images/remote-access.png"
              alt="SimDeck remote access running on iOS"
            />
          </figure>
        </article>

        <article class="sd-moment sd-reveal">
          <div class="sd-index" aria-hidden="true">04</div>
          <div class="sd-moment-copy">
            <h3>A CLI your agents can actually trust</h3>
            <p>
              Boot, install, launch, tap, type, describe, screenshot, record,
              and assert. Stable commands and clear JSON errors turn simulators
              into end-to-end test rigs for every agent in your fleet.
            </p>
          </div>
          <div
            class="sd-shot sd-shot-cli"
            aria-label="Screenshot placeholder for SimDeck CLI automation"
          >
            <div class="sd-cli-lines" aria-hidden="true">
              <span>$ simdeck boot "iPhone 17"</span>
              <span>$ simdeck install app.app</span>
              <span>$ simdeck tap --label Continue</span>
              <span>$ simdeck describe --format agent</span>
            </div>
          </div>
        </article>
      </div>
    </section>

    <section class="sd-close sd-reveal" aria-label="Start using SimDeck">
      <div>
        <h2>Don't switch context. Give the simulator a URL.</h2>
        <p>
          Run it headlessly on your Mac, stream it into your editor or pull
          request, and let agents tap, type, inspect, screenshot, and verify
          without taking over your screen.
        </p>
      </div>
      <a href="/guide/quick-start">Start with the quick start</a>
    </section>
  </main>
</template>
