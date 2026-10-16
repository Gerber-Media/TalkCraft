// TalkCraft product pages: language, downloads for this computer, copy buttons, legal panels,
// tutorial animations.
// English unless the browser prefers German; ?lang=de|en forces one.
(function () {
  var q = new URLSearchParams(location.search).get("lang");
  var prefs = [q].concat(navigator.languages || [], navigator.language).filter(Boolean);
  var lang = "en";
  for (var i = 0; i < prefs.length; i++) {
    var p = String(prefs[i]).slice(0, 2).toLowerCase();
    if (p === "de" || p === "en") { lang = p; break; }
  }
  document.documentElement.lang = lang;
  var root = document.documentElement;
  document.title = (lang === "de" ? root.dataset.titleDe : root.dataset.titleEn) || document.title;
  document.querySelectorAll(".lang a").forEach(function (a) { if (a.dataset.set === lang) a.setAttribute("aria-current", "true"); });
  // The download for this computer first and highlighted.
  var ua = navigator.userAgent, os = /Windows/.test(ua) ? "win" : /Mac OS X|Macintosh/.test(ua) && !/iPhone|iPad/.test(ua) ? "mac" : /Linux/.test(ua) && !/Android/.test(ua) ? "linux" : null;
  var dl = document.getElementById("dl");
  if (os && dl) {
    dl.querySelectorAll(".btn").forEach(function (b) { b.classList.toggle("primary", b.dataset.os === os); });
    var mine = dl.querySelector('[data-os="' + os + '"]');
    if (mine) dl.insertBefore(mine, dl.firstChild);
    // First-launch help for this computer open, the others collapsed.
    var help = document.querySelector('.howto [data-os="' + os + '"]');
    if (help) { help.open = true; help.parentNode.insertBefore(help, help.parentNode.firstChild); }
    // Only this computer's install command in the hero.
    var shell = os === "win" ? "win" : "unix";
    document.querySelectorAll(".install [data-shell]").forEach(function (d) { d.hidden = d.dataset.shell !== shell; });
  }
  // A copy button on every command.
  var label = lang === "de" ? ["Kopieren", "Kopiert ✔"] : ["Copy", "Copied ✔"];
  document.querySelectorAll("pre").forEach(function (pre) {
    var box = document.createElement("div"), btn = document.createElement("button");
    box.className = "cmd";
    btn.type = "button"; btn.className = "copy"; btn.textContent = label[0];
    pre.parentNode.insertBefore(box, pre);
    box.appendChild(pre); box.appendChild(btn);
    btn.addEventListener("click", function () {
      var text = pre.textContent.trim(), done = function () {
        btn.textContent = label[1]; btn.classList.add("done");
        setTimeout(function () { btn.textContent = label[0]; btn.classList.remove("done"); }, 1800);
      };
      if (navigator.clipboard && window.isSecureContext) {
        navigator.clipboard.writeText(text).then(done, function () { select(pre); });
      } else { select(pre); if (document.execCommand("copy")) done(); }
    });
  });
  // Impressum and privacy: two buttons side by side, one panel open at a time (#impressum opens it).
  var legal = document.querySelectorAll(".legal-nav button");
  function showLegal(id) {
    legal.forEach(function (b) {
      var on = b.getAttribute("aria-controls") === id && b.getAttribute("aria-expanded") !== "true";
      b.setAttribute("aria-expanded", on ? "true" : "false");
      document.getElementById(b.getAttribute("aria-controls")).hidden = !on;
    });
  }
  legal.forEach(function (b) { b.addEventListener("click", function () { showLegal(b.getAttribute("aria-controls")); }); });
  if (location.hash === "#impressum" || location.hash === "#datenschutz") {
    showLegal(location.hash.slice(1));
    document.getElementById(location.hash.slice(1)).scrollIntoView();
  }
  // Tutorial animations: the GIF in this page's language; a click pauses (shows the still) or plays;
  // with reduced motion they start as stills.
  var still = window.matchMedia && matchMedia("(prefers-reduced-motion: reduce)").matches;
  document.querySelectorAll("figure.anim").forEach(function (fig) {
    var img = fig.querySelector("img"), base = fig.dataset.src + "-" + lang;
    var play = function (on) {
      img.src = base + (on ? ".gif" : ".png");
      fig.classList.toggle("paused", !on);
    };
    play(!still);
    fig.addEventListener("click", function () { play(fig.classList.contains("paused")); });
    fig.setAttribute("title", lang === "de" ? "Klicken: anhalten oder abspielen" : "Click to pause or play");
  });
  function select(el) { var r = document.createRange(); r.selectNodeContents(el); var s = getSelection(); s.removeAllRanges(); s.addRange(r); }
})();
