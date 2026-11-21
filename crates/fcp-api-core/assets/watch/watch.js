// A webinar's watch page (FCP docs/WEBINARS.md W8): the stream played from
// this page's playlist link, natively where the browser plays HLS (Safari),
// through hls.js elsewhere; "waiting" until there is a stream, tried again
// every ten seconds.
(function () {
  "use strict";
  var video = document.getElementById("video");
  var state = document.getElementById("state");
  var playlist = document.body.getAttribute("data-playlist");
  var hls = null;
  var retry = null;

  function say(text) { state.textContent = text; }

  function later() {
    if (retry) return;
    retry = setTimeout(function () { retry = null; start(); }, 10000);
  }

  function start() {
    fetch(playlist, { cache: "no-store" }).then(function (r) {
      if (r.status === 404) { say("Waiting for the webinar to start…"); later(); return; }
      if (r.status === 401 || r.status === 403) {
        say("This link no longer opens the webinar.");
        return;
      }
      if (!r.ok) { say("The webinar cannot be reached just now; trying again…"); later(); return; }
      return r.text().then(function (text) {
        var ended = text.indexOf("#EXT-X-ENDLIST") >= 0;
        say(ended ? "The webinar has ended: this is its recording." : "Live");
        play();
      });
    }).catch(function () { say("The webinar cannot be reached just now; trying again…"); later(); });
  }

  function play() {
    if (video.canPlayType("application/vnd.apple.mpegurl")) {
      video.src = playlist;
      video.play().catch(function () {});
      return;
    }
    if (!window.Hls || !window.Hls.isSupported()) {
      say("This browser cannot play the webinar.");
      return;
    }
    if (hls) hls.destroy();
    hls = new window.Hls({ liveDurationInfinity: true });
    hls.on(window.Hls.Events.ERROR, function (_, data) {
      if (data.fatal) { hls.destroy(); hls = null; say("The stream was lost; trying again…"); later(); }
    });
    hls.loadSource(playlist);
    hls.attachMedia(video);
    hls.on(window.Hls.Events.MANIFEST_PARSED, function () { video.play().catch(function () {}); });
  }

  start();
})();
