import io
import zipfile

import pytest
from fastapi.testclient import TestClient

from paddock_server import versioning
from paddock_server.config import Settings
from paddock_server.main import create_app
from paddock_server.services import clean_filename


# ------------------------------------------------------------------ pure rules

def test_semver():
    assert versioning.next_label("software", None, "minor") == "0.1.0"
    assert versioning.next_label("software", "1.4.2", "patch") == "1.4.3"
    assert versioning.next_label("software", "1.4.2", "minor") == "1.5.0"
    assert versioning.next_label("software", "1.4.2", "major") == "2.0.0"


def test_cad_revisions():
    nl = versioning.next_label
    assert nl("cad", None, "revision") == "A"
    assert nl("cad", "H", "revision") == "J"
    assert nl("cad", "N", "revision") == "P"
    assert nl("cad", "Y", "revision") == "AA"
    assert nl("cad", "B", "iteration") == "B.1"
    assert nl("cad", "B.3", "revision") == "C"
    with pytest.raises(ValueError):
        nl("cad", "A", "minor")


def test_clean_filename_windows_safe():
    assert clean_filename(r"C:\fakepath\manga:v2?.dwg") == "mangav2.dwg"
    assert clean_filename("CON.dwg") == "_CON.dwg"
    assert clean_filename("peça. ") == "peça"


# ------------------------------------------------------------------ API

@pytest.fixture
def app(tmp_path):
    return create_app(Settings.from_env(data_dir=tmp_path))


class Api:
    def __init__(self, app, token=None):
        self.c = TestClient(app)
        self.h = {"Authorization": f"Bearer {token}"} if token else {}

    def __getattr__(self, method):
        def call(url, expect=None, **kw):
            r = getattr(self.c, method)(url, headers=self.h, **kw)
            if expect is not None:
                assert r.status_code == expect, (r.status_code, r.text)
            return r
        return call


def login(app, username, password="senha1234"):
    r = TestClient(app).post("/api/auth/login", json={"username": username, "password": password})
    assert r.status_code == 200, r.text
    return Api(app, r.json()["token"])


@pytest.fixture
def admin(app):
    c = TestClient(app)
    assert c.get("/api/meta").json()["has_users"] is False
    assert "Configuração inicial" in c.get("/").text
    r = c.post("/setup", data={"name": "Capitã", "username": "Capita", "password": "senha1234"})
    assert "Administrador criado" in r.text
    # Setup cannot run twice.
    c.post("/setup", data={"name": "X", "username": "x", "password": "senha1234"})
    assert c.get("/api/meta").json()["has_users"]
    return login(app, "capita")


def make_user(admin, app, username, role):
    admin.post("/api/users", json={"username": username, "name": username.title(), "password": "senha1234",
                                   "role": role}, expect=201)
    return login(app, username)


def new_project(api, kind="cad", name="Suspensão FS-26", **kw):
    return api.post("/api/projects", json={"name": name, "kind": kind, "subsystem": "Suspensão", **kw},
                    expect=201).json()


def new_item(api, project_id, content=b"v1", name="Manga", filename="manga.dwg", **form):
    form = {"name": name, "comment": "primeira", **form}
    return api.post(f"/api/projects/{project_id}/items", data=form,
                    files={"file": (filename, io.BytesIO(content))}, expect=201).json()


def publish(api, item_id, content, expect=201, filename="qualquer.dwg", **form):
    form = {"comment": "mudança", **form}
    return api.post(f"/api/items/{item_id}/versions", data=form,
                    files={"file": (filename, io.BytesIO(content))}, expect=expect).json()


def test_auth(app, admin):
    assert TestClient(app).get("/api/projects").status_code == 401
    assert TestClient(app).post("/api/auth/login", json={"username": "capita", "password": "errada"}).status_code == 401
    assert admin.get("/api/auth/me", expect=200).json()["role"] == "admin"
    admin.post("/api/auth/logout", expect=204)
    admin.get("/api/auth/me", expect=401)


def test_login_throttle(app, admin):
    c = TestClient(app)
    for _ in range(5):
        c.post("/api/auth/login", json={"username": "capita", "password": "x"})
    assert c.post("/api/auth/login", json={"username": "capita", "password": "senha1234"}).status_code == 429


def test_cad_flow_with_checkout(app, admin):
    p = new_project(admin)
    assert p["require_lock"] is True  # default for CAD
    item = new_item(admin, p["id"], part_number="SUS-0142")
    assert item["latest_version"]["label"] == "A"
    assert item["file_name"] == "manga.dwg"
    assert item["next_labels"] == {"revision": "B", "iteration": "A.1"}

    r = publish(admin, item["id"], b"v2", expect=403)
    assert "exige check-out" in r["detail"]

    joao = make_user(admin, app, "joao", "member")
    joao.post(f"/api/items/{item['id']}/lock", json={"note": "furação"}, expect=200)
    assert "Joao" in publish(admin, item["id"], b"v2", expect=403)["detail"]
    assert joao.get("/api/locks").json() == [item["id"]]

    v = publish(joao, item["id"], b"v2", bump="iteration", filename="manga_final_agora_vai.dwg")
    assert v["label"] == "A.1"
    refreshed = joao.get(f"/api/items/{item['id']}").json()
    assert refreshed["locked_by"] is None  # released on publish
    assert refreshed["file_name"] == "manga.dwg"  # stable name

    r = joao.get(f"/api/versions/{v['id']}/download", expect=200)
    assert r.content == b"v2"
    assert "manga_revA.1.dwg" in r.headers["content-disposition"]
    assert r.headers["x-paddock-sha256"] == v["sha256"]
    assert joao.get(f"/api/items/{item['id']}/versions/A/download").content == b"v1"
    assert [x["label"] for x in joao.get(f"/api/items/{item['id']}/versions").json()] == ["A.1", "A"]


def test_identical_and_empty_rejected(admin):
    p = new_project(admin, require_lock=False)
    item = new_item(admin, p["id"], b"same")
    assert "idêntico" in publish(admin, item["id"], b"same", expect=409)["detail"]
    assert "vazio" in publish(admin, item["id"], b"", expect=400)["detail"]
    assert "Descreva" in publish(admin, item["id"], b"x", comment=" ", expect=400)["detail"]


def test_status_permissions(app, admin):
    p = new_project(admin, require_lock=False)
    vid = new_item(admin, p["id"])["latest_version"]["id"]
    ana = make_user(admin, app, "ana", "member")
    ana.patch(f"/api/versions/{vid}", json={"status": "released"}, expect=403)
    ana.patch(f"/api/versions/{vid}", json={"status": "in_review"}, expect=200)
    assert [v["id"] for v in ana.get("/api/reviews").json()] == [vid]
    admin.patch(f"/api/versions/{vid}", json={"status": "released"}, expect=200)
    ana.patch(f"/api/versions/{vid}", json={"status": "draft"}, expect=403)

    leo = make_user(admin, app, "leo", "viewer")
    leo.post("/api/projects", json={"name": "x", "kind": "cad", "subsystem": "Geral"}, expect=403)
    leo.post(f"/api/items/{p['id']}/lock", json={}, expect=403)
    leo.get(f"/api/versions/{vid}/download", expect=200)


def test_force_unlock_and_deactivation(app, admin):
    p = new_project(admin)
    item = new_item(admin, p["id"])
    a = make_user(admin, app, "aa", "member")
    b = make_user(admin, app, "bb", "member")
    a.post(f"/api/items/{item['id']}/lock", json={}, expect=200)
    b.post(f"/api/items/{item['id']}/lock", json={}, expect=409)
    b.delete(f"/api/items/{item['id']}/lock", expect=403)
    admin.delete(f"/api/items/{item['id']}/lock", expect=200)
    actions = [x["action"] for x in admin.get(f"/api/activity?item_id={item['id']}").json()]
    assert actions[0] == "checkout_forced"

    a.post(f"/api/items/{item['id']}/lock", json={}, expect=200)
    uid = a.get("/api/auth/me").json()["id"]
    admin.patch(f"/api/users/{uid}", json={"active": False}, expect=200)
    a.get("/api/auth/me", expect=401)
    assert admin.get(f"/api/items/{item['id']}").json()["locked_by"] is None


def test_admin_cannot_demote_self(admin):
    me = admin.get("/api/auth/me").json()
    admin.patch(f"/api/users/{me['id']}", json={"role": "member"}, expect=400)


def test_baseline(admin):
    p = new_project(admin, require_lock=False)
    i1 = new_item(admin, p["id"], b"manga", "Manga")
    new_item(admin, p["id"], b"bandeja", "Bandeja", filename="bandeja.step")
    publish(admin, i1["id"], b"manga-b")
    b = admin.post(f"/api/projects/{p['id']}/baselines", json={"name": "FSAE Brasil 2026"}, expect=201).json()
    publish(admin, i1["id"], b"manga-c")  # later versions don't change the baseline
    detail = admin.get(f"/api/baselines/{b['id']}").json()
    assert sorted((e["item_name"], e["version"]["label"]) for e in detail["entries"]) == [("Bandeja", "A"), ("Manga", "B")]
    z = zipfile.ZipFile(io.BytesIO(admin.get(f"/api/baselines/{b['id']}/zip", expect=200).content))
    assert z.read("Manga/manga_revB.dwg") == b"manga-b"
    assert "MANIFESTO.txt" in z.namelist()
    versions = admin.get(f"/api/items/{i1['id']}/versions").json()
    assert versions[1]["baselines"][0]["name"] == "FSAE Brasil 2026"

    only = admin.post(f"/api/projects/{p['id']}/baselines", json={"name": "x", "only_released": True})
    assert only.status_code == 400


def test_software_and_ci_token(admin):
    p = new_project(admin, kind="software", name="Firmware ECU")
    assert p["require_lock"] is False
    item = new_item(admin, p["id"], b"\x01", "ECU", filename="ecu.hex")
    assert item["latest_version"]["label"] == "0.1.0"
    t = admin.post("/api/auth/tokens", json={"name": "GitHub Actions"}, expect=201).json()
    ci = Api(admin.c.app, t["token"])
    v = publish(ci, item["id"], b"\x02", bump="patch", filename="ecu.hex")
    assert v["label"] == "0.1.1"
    assert ci.get(f"/api/items/{item['id']}/versions/latest/download").content == b"\x02"
    ci.get(f"/api/items/{item['id']}/versions/latest/download?status=released", expect=404)
    admin.delete(f"/api/auth/tokens/{t['id']}", expect=204)
    ci.get("/api/auth/me", expect=401)


def test_search_and_verify(admin):
    p = new_project(admin, require_lock=False)
    item = new_item(admin, p["id"], part_number="SUS-0999")
    assert admin.get("/api/search?q=sus-09").json()["items"][0]["id"] == item["id"]
    assert admin.get(f"/api/items/{item['id']}/verify").json() == {"ok": True, "corrupted": []}


def test_downloads_page_no_traversal(app, admin, tmp_path):
    (tmp_path / "downloads" / "paddock.exe").write_bytes(b"MZ")
    c = TestClient(app)
    assert "paddock.exe" in c.get("/").text
    assert c.get("/downloads/paddock.exe").content == b"MZ"
    assert c.get("/downloads/..%2Fpaddock.sqlite3").status_code == 404


def test_duplicate_file_name_in_project(admin):
    p = new_project(admin, require_lock=False)
    new_item(admin, p["id"], b"1", "Manga esquerda", filename="manga.dwg")
    r = admin.post(f"/api/projects/{p['id']}/items", data={"name": "Manga direita", "comment": "x"},
                   files={"file": ("MANGA.dxf", io.BytesIO(b"2"))})
    assert r.status_code == 409 and "manga" in r.json()["detail"].lower()
