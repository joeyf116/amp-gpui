// The tone generator's interface. Inside a Claude artifact it asks Claude
// directly, on the viewer's own account. On the docs site, where a page can't
// use a Claude login, it offers a copied prompt or the visitor's own API key.
(() => {
  const SAMPLE_IDEAS = [
    "Warm edge-of-breakup blues for a Strat",
    "Tight modern high gain rhythm, humbuckers",
    "Glassy clean with slow chorus and a long delay",
    "The lead tone from a big 80s rock ballad",
  ];

  const MODELS = [
    ["claude-opus-5-5", "Claude Opus 5.5"],
    ["claude-sonnet-5-5", "Claude Sonnet 5.5"],
    ["claude-haiku-5-5", "Claude Haiku 5.5"],
  ];

  // The published artifact, which runs on claude.ai.
  const ARTIFACT_URL = "https://claude.ai/artifact/PcgLuALJ8RZhQn4GTY9p7W";

  const SAMPLE_MESSAGES = {
    not_granted: "This page needs your OK to use Claude. Reload it to be asked again.",
    sampling_disabled: "Claude isn't available on this account.",
    rate_limited: "You've reached a usage limit. Try again in a little while.",
    session_expired: "Sign in to Claude again, then retry.",
    refused: "Claude declined that description. Try describing it differently.",
    invalid_json: "Claude's reply wasn't a complete preset. Try again.",
  };

  function element(tag, properties = {}, ...children) {
    const node = Object.assign(document.createElement(tag), properties);
    node.append(...children);
    return node;
  }

  function button(label, onclick, className = "") {
    return element("button", { type: "button", textContent: label, onclick, className: `gen-button ${className}` });
  }

  function saveInBrowser(filename, data, type) {
    const url = URL.createObjectURL(new Blob([data], { type }));
    const link = element("a", { href: url, download: filename });
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 10_000);
  }

  async function askWithApiKey({ key, model, description, onText, signal }) {
    const url = new URL("anthropic-sdk.js", document.querySelector("script[src*='generator/ui.js']").src);
    const { default: Anthropic } = await import(url);
    const client = new Anthropic({ apiKey: key, dangerouslyAllowBrowser: true });
    // Haiku has no server-side fallback for a refused request.
    const fallback =
      model === "claude-haiku-5-5"
        ? {}
        : { betas: ["server-side-fallback-2026-07-01"], fallbacks: "default" };
    const stream = client.beta.messages.stream(
      {
        model,
        max_tokens: 16000,
        ...fallback,
        output_config: {
          effort: "medium",
          format: { type: "json_schema", schema: AmpTones.responseSchema() },
        },
        system: AmpTones.instructions(),
        messages: [{ role: "user", content: AmpTones.request(description) }],
      },
      { signal },
    );
    stream.on("text", onText);
    try {
      const message = await stream.finalMessage();
      if (message.stop_reason === "refusal") {
        throw new Error("Claude declined that description. Try describing it differently.");
      }
      if (message.stop_reason === "max_tokens") {
        throw new Error("The reply was cut off before the preset was finished. Try again.");
      }
      return message.content
        .filter((block) => block.type === "text")
        .map((block) => block.text)
        .join("");
    } catch (error) {
      if (error instanceof Anthropic.AuthenticationError) {
        throw new Error("Anthropic didn't accept that API key.");
      }
      if (error instanceof Anthropic.PermissionDeniedError) {
        throw new Error("That API key can't use this model. Try another one.");
      }
      if (error instanceof Anthropic.RateLimitError) {
        throw new Error("That key is rate limited or out of credit. Check the Claude Console.");
      }
      if (error instanceof Anthropic.APIConnectionError) {
        throw new Error("Couldn't reach Anthropic. Check your connection and try again.");
      }
      if (error instanceof Anthropic.APIError) {
        throw new Error(`Anthropic returned an error (${error.status ?? "no status"}): ${error.message}`);
      }
      throw error;
    }
  }

  function mount(root, sample, downloads) {
    const description = element("textarea", {
      id: "gen-description",
      rows: 3,
      maxLength: 1000,
      placeholder: "Describe a tone, a song, or a player. Mention your guitar if it matters.",
    });
    const status = element("p", { className: "gen-status", role: "status" });
    const result = element("div", { className: "gen-result", hidden: true });
    let controller = null;

    const ideas = element("div", { className: "gen-ideas" });
    for (const idea of SAMPLE_IDEAS) {
      ideas.append(button(idea, () => { description.value = idea; description.focus(); }, "gen-idea"));
    }

    const setStatus = (text, isError = false) => {
      status.textContent = text;
      status.classList.toggle("gen-error", isError);
    };

    async function save(filename, data, type) {
      if (!downloads) return saveInBrowser(filename, data, type);
      try {
        await downloads.save({ filename, data });
      } catch (error) {
        if (error.code !== "declined") setStatus("This view can't save files. Open the page in claude.ai.", true);
      }
    }

    function show({ preset, warnings }) {
      const chain = element("ol", { className: "gen-chain" });
      for (const { title, settings } of AmpTones.describeChain(preset)) {
        chain.append(element("li", {}, element("strong", { textContent: title }), element("span", { textContent: settings.join(" · ") })));
      }
      const pack = AmpTones.tonePack(preset);
      result.replaceChildren(
        element("h3", { textContent: preset.name }),
        element("p", { className: "gen-tags", textContent: preset.tags.join(" · ") }),
        element("p", { textContent: preset.notes }),
        ...warnings.map((text) => element("p", { className: "gen-error", textContent: text })),
        chain,
        element(
          "div",
          { className: "gen-actions" },
          button("Download tone pack", () => save(pack.filename, pack.data, "application/zip"), "gen-primary"),
          button("Download preset JSON", () => save(`${AmpTones.slug(preset.name)}.json`, AmpTones.presetJson(preset), "application/json")),
        ),
        element("p", {
          className: "gen-hint",
          textContent: "To add it: unzip the pack, press Import under the tone list in Amp GPUI, and choose the unzipped folder. Nobody has heard this tone yet, so start with your volume low.",
        }),
      );
      result.hidden = false;
      result.scrollIntoView({ block: "nearest" });
      setStatus("");
    }

    function accept(text) {
      try {
        show(AmpTones.normalize(AmpTones.parseReply(text)));
      } catch (error) {
        setStatus(error.message, true);
      }
    }

    function wanted() {
      if (description.value.trim()) return description.value;
      setStatus("Describe the tone you want first.", true);
      description.focus();
      return null;
    }

    function generator(run) {
      const go = button("Generate tone", async () => {
        const text = wanted();
        if (!text) return;
        controller = new AbortController();
        go.disabled = true;
        stop.hidden = false;
        result.hidden = true;
        setStatus("Thinking...");
        try {
          await run(text, controller.signal);
        } catch (error) {
          const stopped = error.code === "cancelled" || error.name === "AbortError" || controller.signal.aborted;
          setStatus(stopped ? "" : error.message, !stopped);
        } finally {
          go.disabled = false;
          stop.hidden = true;
        }
      }, "gen-primary");
      const stop = button("Stop", () => controller?.abort());
      stop.hidden = true;
      return element("div", { className: "gen-actions" }, go, stop);
    }

    const prompt = element("label", { className: "gen-label", htmlFor: "gen-description", textContent: "Tone" });

    if (sample) {
      const run = async (text, signal) => {
        try {
          const reply = await sample(AmpTones.fullPrompt(text), {
            cache: false,
            signal,
            onText: () => setStatus("Writing the preset..."),
          });
          accept(reply.text);
        } catch (error) {
          throw new Error(SAMPLE_MESSAGES[error.code] ?? "Something went wrong reaching Claude. Try again.", { cause: error });
        }
      };
      root.replaceChildren(prompt, description, ideas, generator(run), status, result);
      return;
    }

    if (globalThis.claude) {
      root.replaceChildren(element("p", { textContent: "Open this page in claude.ai to generate tones." }));
      return;
    }

    const reply = element("textarea", { id: "gen-reply", rows: 4, placeholder: "Paste Claude's whole reply here." });
    const pasteRoute = element(
      "div",
      { className: "gen-route" },
      element("h3", { textContent: "Copy a prompt into Claude" }),
      element("p", { textContent: "Works with any Claude plan, including Free. Copy the prompt, send it in a new chat, then paste the reply below." }),
      element(
        "div",
        { className: "gen-actions" },
        button("Copy prompt", async () => {
          const text = wanted();
          if (!text) return;
          await navigator.clipboard.writeText(AmpTones.fullPrompt(text));
          setStatus("Prompt copied. Paste it into a new Claude chat.");
        }, "gen-primary"),
        element("a", { className: "gen-button", href: "https://claude.ai/new", target: "_blank", rel: "noopener", textContent: "Open Claude" }),
      ),
      reply,
      element("div", { className: "gen-actions" }, button("Use this reply", () => accept(reply.value))),
    );

    const key = element("input", {
      id: "gen-key",
      type: "password",
      autocomplete: "off",
      spellcheck: false,
      placeholder: "sk-ant-...",
    });
    const model = element("select", { id: "gen-model" });
    for (const [id, label] of MODELS) model.append(element("option", { value: id, textContent: label }));
    const keyRoute = element(
      "div",
      { className: "gen-route" },
      element("h3", { textContent: "Use your own API key" }),
      element("p", {
        textContent: "API keys come from the Claude Console and are billed by usage, separately from a Claude plan. This page sends your key only to api.anthropic.com and never saves it, so it's gone when you leave. Use a key you can revoke.",
      }),
      element("label", { className: "gen-label", htmlFor: "gen-key", textContent: "API key" }),
      key,
      element("label", { className: "gen-label", htmlFor: "gen-model", textContent: "Model" }),
      model,
      generator(async (text, signal) => {
        if (!key.value.trim()) throw new Error("Enter an API key first.");
        const reply = await askWithApiKey({
          key: key.value.trim(),
          model: model.value,
          description: text,
          signal,
          onText: () => setStatus("Writing the preset..."),
        });
        accept(reply);
      }),
    );

    const accountRoute = element(
      "div",
      { className: "gen-route" },
      element("h3", { textContent: "Sign in with your Claude account" }),
      element("p", { textContent: "The easiest way. The generator opens in Claude, and any use counts toward your own plan. No API key needed." }),
      element("div", { className: "gen-actions" }, element("a", { className: "gen-button gen-primary", href: ARTIFACT_URL, target: "_blank", rel: "noopener", textContent: "Open the generator in Claude" })),
    );

    root.replaceChildren(prompt, description, ideas, accountRoute, pasteRoute, keyRoute, status, result);
  }

  async function start() {
    const root = document.getElementById("tone-generator");
    if (!root || root.dataset.ready) return;
    root.dataset.ready = "true";
    const [sample, downloads] = globalThis.claude
      ? await Promise.all([claude.use("sample"), claude.use("downloads")])
      : [null, null];
    mount(root, sample, downloads);
  }

  // The site swaps pages without a reload, so this runs on each one.
  if (globalThis.document$) document$.subscribe(start);
  else start();
})();
