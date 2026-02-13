self.addEventListener("install", (event) => {
  event.waitUntil(self.skipWaiting());
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const targetUrl = event.notification.data?.url || "/";
  const targetPath = new URL(targetUrl, self.location.origin);

  event.waitUntil(
    (async () => {
      const clients = await self.clients.matchAll({
        type: "window",
        includeUncontrolled: true,
      });

      for (const client of clients) {
        if ("focus" in client) {
          const clientUrl = new URL(client.url);
          if (clientUrl.pathname === targetPath.pathname && clientUrl.origin === targetPath.origin) {
            await client.focus();
            if (clientUrl.href !== targetPath.href && "navigate" in client) {
              await client.navigate(targetPath.href);
            }
            return;
          }
        }
      }

      if (self.clients.openWindow) {
        await self.clients.openWindow(targetUrl);
      }
    })(),
  );
});

self.addEventListener("push", (event) => {
  const fallback = {
    title: "Cleo",
    body: "You have new content ready.",
    data: { url: "/" },
    tag: "cleo-content",
  };

  event.waitUntil(
    (async () => {
      let payload = fallback;
      try {
        if (event.data) {
          const parsed = event.data.json();
          payload = {
            title: parsed.title || fallback.title,
            body: parsed.body || fallback.body,
            data: parsed.data || fallback.data,
            tag: parsed.tag || fallback.tag,
          };
        }
      } catch {
        // Ignore and use fallback notification text.
      }

      await self.registration.showNotification(payload.title, {
        body: payload.body,
        icon: "/icon-192.png",
        badge: "/icon-192.png",
        data: payload.data,
        tag: payload.tag,
        renotify: true,
        requireInteraction: false,
        vibrate: [80, 40, 80],
      });
    })(),
  );
});
