// CleanClaw WebSocket chat client. Vanilla JS, no framework.
//
// Loaded by the /agents/:id/sessions/:sid page (the chat
// surface). The page embeds a small DOM contract the client
// expects:
//
//   <div class="chat-surface" data-agent-id="..." data-session-id="...">
//     <div class="chat-header">...</div>
//     <div class="chat-history" data-chat-history>
//       ...pre-rendered <div class="chat-msg" data-msg-id="N"> rows
//     </div>
//     <form class="chat-composer" data-chat-form>
//       <textarea data-chat-input></textarea>
//       <button data-chat-send>Send</button>
//     </form>
//     <span data-chat-status>idle</span>
//   </div>
//
// The client:
//   1. Reads the agent_id + session_id from data-* attributes
//   2. Reads the apikey cookie (`cleanclaw_session` OR a
//      bearer token in localStorage under `cleanclaw_apikey`)
//   3. Opens a WS to /api/ws/chat
//   4. Sends `connect` with the token, then `chat.send` on form
//      submit
//   5. Renders each `chat.<event>` frame inline: deltas
//      append to a streaming "assistant" bubble, tool calls
//      show a small sub-card, done/error update the status
//      span and re-enable the form
//
// The page works without JS (the pre-rendered history is the
// fallback). The client is a progressive enhancement.

(function () {
  'use strict';

  function findSurface() {
    return document.querySelector('[data-chat-history]');
      ? document.querySelector('.chat-surface')
      : null;
  }

  function getToken() {
    // Prefer the session cookie set by the SSR login flow.
    // Falls back to the apikey in localStorage (used by the
    // dashboard for direct API calls).
    var m = document.cookie.match(/cleanclaw_session=([^;]+)/);
    if (m) return m[1];
    try {
      return localStorage.getItem('cleanclaw_apikey') || '';
    } catch (_) {
      return '';
    }
  }

  function setStatus(surface, text) {
    var el = surface.querySelector('[data-chat-status]');
    if (el) el.textContent = text;
  }

  function findOrCreateStreamingBubble(history) {
    // The last child is a streaming bubble if it has the
    // `data-streaming` attribute. Otherwise create a new one
    // and append.
    var last = history.lastElementChild;
    if (last && last.getAttribute('data-streaming') === '1') return last;
    var bubble = document.createElement('div');
    bubble.className = 'chat-msg chat-msg-assistant chat-msg-streaming';
    bubble.setAttribute('data-streaming', '1');
    bubble.innerHTML =
      '<div class="chat-msg-role">assistant</div>' +
      '<div class="chat-msg-body"></div>';
    history.appendChild(bubble);
    history.scrollTop = history.scrollHeight;
    return bubble;
  }

  function appendText(bubble, text) {
    var body = bubble.querySelector('.chat-msg-body');
    if (!body) return;
    body.textContent = (body.textContent || '') + text;
    var history = bubble.parentElement;
    if (history) history.scrollTop = history.scrollHeight;
  }

  function appendToolCard(bubble, name, args) {
    var pre = document.createElement('pre');
    pre.className = 'chat-tool-pre';
    try {
      pre.textContent = name + '(' + JSON.stringify(args, null, 2) + ')';
    } catch (_) {
      pre.textContent = name + '(...)';
    }
    bubble.appendChild(pre);
  }

  function commitStreaming(bubble) {
    if (!bubble) return;
    bubble.removeAttribute('data-streaming');
    bubble.classList.remove('chat-msg-streaming');
  }

  function startSession(surface) {
    var history = surface.querySelector('[data-chat-history]');
    var form = surface.querySelector('[data-chat-form]');
    var input = surface.querySelector('[data-chat-input]');
    var send = surface.querySelector('[data-chat-send]');
    var agentId = surface.getAttribute('data-agent-id');
    var sessionId = surface.getAttribute('data-session-id');
    if (!history || !form || !input || !send) return;

    var ws = null;
    var nextReqId = 1;
    var streamingBubble = null;

    function sendFrame(obj) {
      if (!ws || ws.readyState !== WebSocket.OPEN) return false;
      ws.send(JSON.stringify(obj));
      return true;
    }

    function openWs() {
      var proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
      var url = proto + '//' + location.host + '/api/ws/chat';
      ws = new WebSocket(url);

      ws.addEventListener('open', function () {
        setStatus(surface, 'connecting');
        var token = getToken();
        if (!token) {
          setStatus(surface, 'not signed in');
          return;
        }
        sendFrame({
          type: 'req',
          id: 'c1',
          method: 'connect',
          params: { auth: { token: token } },
        });
      });

      ws.addEventListener('message', function (ev) {
        var f;
        try { f = JSON.parse(ev.data); } catch (_) { return; }
        if (f.type === 'res' && f.id === 'c1') {
          if (f.ok) {
            setStatus(surface, 'ready');
            input.disabled = false;
            send.disabled = false;
          } else {
            setStatus(surface, 'auth failed: ' + (f.error && f.error.message || 'unknown'));
          }
        } else if (f.type === 'event') {
          var ev2 = f.event;
          var pl = f.payload || {};
          if (ev2 === 'chat.start') {
            setStatus(surface, 'thinking…');
            input.disabled = true;
            send.disabled = true;
            streamingBubble = findOrCreateStreamingBubble(history);
          } else if (ev2 === 'chat.delta') {
            if (streamingBubble) appendText(streamingBubble, pl.delta || '');
          } else if (ev2 === 'chat.thinking') {
            // For now, surface thinking in the status line. A
            // future UI will fold it into the bubble.
            setStatus(surface, 'thinking: ' + (pl.delta || ''));
          } else if (ev2 === 'chat.tool_call') {
            if (streamingBubble)
              appendToolCard(streamingBubble, pl.name, pl.arguments);
          } else if (ev2 === 'chat.tool_result') {
            // Tool result is informational; the JS layer
            // surfaces it as a small badge next to the tool
            // call card.
            if (streamingBubble) {
              var badge = document.createElement('div');
              badge.className = 'chat-tool-result';
              badge.textContent =
                '→ ' + (pl.id || '') + ' ' + (pl.is_error ? '✗' : '✓');
              streamingBubble.appendChild(badge);
            }
          } else if (ev2 === 'chat.done') {
            setStatus(surface, 'done (' + (pl.finish_reason || '?') + ')');
            commitStreaming(streamingBubble);
            streamingBubble = null;
            input.disabled = false;
            send.disabled = false;
            input.value = '';
            // Persist the new user message URL — the user
            // can refresh and see the same history.
            input.focus();
          } else if (ev2 === 'chat.error') {
            setStatus(surface, 'error: ' + (pl.message || 'unknown'));
            commitStreaming(streamingBubble);
            streamingBubble = null;
            input.disabled = false;
            send.disabled = false;
          }
        }
      });

      ws.addEventListener('close', function () {
        setStatus(surface, 'disconnected (will retry)');
        setTimeout(function () { if (form.isConnected) openWs(); }, 2000);
      });

      ws.addEventListener('error', function () {
        setStatus(surface, 'connection error');
      });
    }

    form.addEventListener('submit', function (e) {
      e.preventDefault();
      var msg = (input.value || '').trim();
      if (!msg) return;
      if (!ws || ws.readyState !== WebSocket.OPEN) {
        setStatus(surface, 'not connected');
        return;
      }
      // Optimistically render the user message so the page
      // feels instant. The server's persistence will catch up
      // on the next page load.
      var userMsg = document.createElement('div');
      userMsg.className = 'chat-msg chat-msg-user';
      userMsg.innerHTML =
        '<div class="chat-msg-role">you</div>' +
        '<div class="chat-msg-body"></div>';
      userMsg.querySelector('.chat-msg-body').textContent = msg;
      history.appendChild(userMsg);
      history.scrollTop = history.scrollHeight;
      input.value = '';
      setStatus(surface, 'sending…');
      sendFrame({
        type: 'req',
        id: 'r' + (nextReqId++),
        method: 'chat.send',
        params: {
          agent_id: agentId,
          message: msg,
          session_key: sessionId,
        },
      });
    });

    openWs();
  }

  function init() {
    var surface = findSurface();
    if (!surface) return;
    startSession(surface);
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
