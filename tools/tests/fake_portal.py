#!/usr/bin/env python3
"""Minimal xdg-desktop-portal GlobalShortcuts and a notification server for tests.

Test control on `org.macafon.Test`: Press(id), Release(id), Bound() -> JSON of the last bind,
Notifications() -> JSON list of notification summaries.
"""

import json

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

PORTAL = "org.freedesktop.portal.GlobalShortcuts"
PATH = "/org/freedesktop/portal/desktop"


class Request(dbus.service.Object):
    @dbus.service.signal("org.freedesktop.portal.Request", signature="ua{sv}")
    def Response(self, code, results):
        pass


class Session(dbus.service.Object):
    @dbus.service.method("org.freedesktop.portal.Session", in_signature="", out_signature="")
    def Close(self):
        self.remove_from_connection()


class Portal(dbus.service.Object):
    def __init__(self, bus):
        super().__init__(bus, PATH)
        self.bus = bus
        self.session = None
        self.bound = []
        self.notifications = []

    def respond(self, sender, options, results):
        token = str(options["handle_token"])
        path = f"/org/freedesktop/portal/desktop/request/{sender[1:].replace('.', '_')}/{token}"
        request = Request(self.bus, path)
        GLib.timeout_add(20, lambda: (request.Response(dbus.UInt32(0), results), request.remove_from_connection())[0] and False)
        return dbus.ObjectPath(path)

    @dbus.service.method("org.freedesktop.DBus.Properties", in_signature="ss", out_signature="v")
    def Get(self, interface, name):
        return dbus.UInt32(1)

    @dbus.service.method("org.freedesktop.DBus.Properties", in_signature="s", out_signature="a{sv}")
    def GetAll(self, interface):
        return {"version": dbus.UInt32(1)}

    @dbus.service.method(PORTAL, in_signature="a{sv}", out_signature="o", sender_keyword="sender")
    def CreateSession(self, options, sender=None):
        path = f"/org/freedesktop/portal/desktop/session/{sender[1:].replace('.', '_')}/{options['session_handle_token']}"
        if self.session:
            try:
                self.session.remove_from_connection()
            except LookupError:
                pass
        self.session = Session(self.bus, path)
        self.session_path = path
        return self.respond(sender, options, {"session_handle": dbus.String(path)})

    @dbus.service.method(PORTAL, in_signature="oa(sa{sv})sa{sv}", out_signature="o", sender_keyword="sender")
    def BindShortcuts(self, session, shortcuts, parent, options, sender=None):
        self.bound = [{"id": str(id_), "description": str(props.get("description", "")),
                       "trigger": str(props.get("preferred_trigger", ""))} for id_, props in shortcuts]
        result = dbus.Array([dbus.Struct((dbus.String(item["id"]), dbus.Dictionary(
            {"description": dbus.String(item["description"]),
             "trigger_description": dbus.String(item["trigger"])}, signature="sv")))
            for item in self.bound], signature="(sa{sv})")
        return self.respond(sender, options, {"shortcuts": result})

    @dbus.service.signal(PORTAL, signature="osta{sv}")
    def Activated(self, session, shortcut, timestamp, options):
        pass

    @dbus.service.signal(PORTAL, signature="osta{sv}")
    def Deactivated(self, session, shortcut, timestamp, options):
        pass

    @dbus.service.method("org.macafon.Test", in_signature="s", out_signature="")
    def Press(self, shortcut):
        self.Activated(dbus.ObjectPath(self.session_path), shortcut, dbus.UInt64(0), dbus.Dictionary({}, signature="sv"))

    @dbus.service.method("org.macafon.Test", in_signature="s", out_signature="")
    def Release(self, shortcut):
        self.Deactivated(dbus.ObjectPath(self.session_path), shortcut, dbus.UInt64(0), dbus.Dictionary({}, signature="sv"))

    @dbus.service.method("org.macafon.Test", in_signature="", out_signature="s")
    def Bound(self):
        return json.dumps(self.bound)

    @dbus.service.method("org.macafon.Test", in_signature="", out_signature="s")
    def Notifications(self):
        return json.dumps(self.notifications)


class Notifications(dbus.service.Object):
    def __init__(self, bus, portal):
        super().__init__(bus, "/org/freedesktop/Notifications")
        self.portal = portal

    @dbus.service.method("org.freedesktop.Notifications", in_signature="susssasa{sv}i", out_signature="u")
    def Notify(self, app, replaces, icon, summary, body, actions, hints, timeout):
        self.portal.notifications.append(str(summary))
        return dbus.UInt32(replaces or len(self.portal.notifications))


def main():
    DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    portal = Portal(bus)
    Notifications(bus, portal)
    names = [dbus.service.BusName(name, bus) for name in
             ("org.freedesktop.portal.Desktop", "org.freedesktop.Notifications")]
    print("ready", flush=True)
    GLib.MainLoop().run()
    del names


if __name__ == "__main__":
    main()
