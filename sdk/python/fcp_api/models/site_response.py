from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.site_status import SiteStatus
from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.site_node import SiteNode
  from ..models.site_registrations_view import SiteRegistrationsView
  from ..models.site_response_versions import SiteResponseVersions





T = TypeVar("T", bound="SiteResponse")



@_attrs_define
class SiteResponse:
    """ A site as the API shows it.

        Attributes:
            id (str):
            tenant_id (str):
            name (str):
            status (SiteStatus): Where a site is in its life.
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            link_up (bool):
            versions (SiteResponseVersions): Release → node ids, from the last heartbeat.
            nodes (list[SiteNode]): The site's nodes as the link last found them.
            enrolled_at (datetime.datetime | None | Unset):
            certificate_expires_at (datetime.datetime | None | Unset):
            last_seen_at (datetime.datetime | None | Unset):
            link_version (None | str | Unset):
            sync_cursor (int | None | Unset): The journal sequence the site has applied through, from its last
                heartbeat; absent from a link that keeps no site database.
            sync_applied_at (datetime.datetime | None | Unset): When the site last applied a snapshot or a change.
            sync_error (None | str | Unset): The site's last failure to fetch or apply, until its next success.
            sync_behind (int | None | Unset): Journal entries the site has not applied yet, as of this answer.
            trunk_id (None | str | Unset): The site's link trunk, made at enrollment when the cloud names a
                SIP address for sites (`docs/HYBRID_CONTROL_PLANE.md` §3.6).
            registrations (None | SiteRegistrationsView | Unset):
     """

    id: str
    tenant_id: str
    name: str
    status: SiteStatus
    created_at: datetime.datetime
    updated_at: datetime.datetime
    link_up: bool
    versions: SiteResponseVersions
    nodes: list[SiteNode]
    enrolled_at: datetime.datetime | None | Unset = UNSET
    certificate_expires_at: datetime.datetime | None | Unset = UNSET
    last_seen_at: datetime.datetime | None | Unset = UNSET
    link_version: None | str | Unset = UNSET
    sync_cursor: int | None | Unset = UNSET
    sync_applied_at: datetime.datetime | None | Unset = UNSET
    sync_error: None | str | Unset = UNSET
    sync_behind: int | None | Unset = UNSET
    trunk_id: None | str | Unset = UNSET
    registrations: None | SiteRegistrationsView | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.site_node import SiteNode # noqa: PLC0415
        from ..models.site_registrations_view import SiteRegistrationsView # noqa: PLC0415
        from ..models.site_response_versions import SiteResponseVersions # noqa: PLC0415
        id = self.id

        tenant_id = self.tenant_id

        name = self.name

        status = self.status.value

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        link_up = self.link_up

        versions = self.versions.to_dict()

        nodes = []
        for nodes_item_data in self.nodes:
            nodes_item = nodes_item_data.to_dict()
            nodes.append(nodes_item)



        enrolled_at: None | str | Unset
        if isinstance(self.enrolled_at, Unset):
            enrolled_at = UNSET
        elif isinstance(self.enrolled_at, datetime.datetime):
            enrolled_at = self.enrolled_at.isoformat()
        else:
            enrolled_at = self.enrolled_at

        certificate_expires_at: None | str | Unset
        if isinstance(self.certificate_expires_at, Unset):
            certificate_expires_at = UNSET
        elif isinstance(self.certificate_expires_at, datetime.datetime):
            certificate_expires_at = self.certificate_expires_at.isoformat()
        else:
            certificate_expires_at = self.certificate_expires_at

        last_seen_at: None | str | Unset
        if isinstance(self.last_seen_at, Unset):
            last_seen_at = UNSET
        elif isinstance(self.last_seen_at, datetime.datetime):
            last_seen_at = self.last_seen_at.isoformat()
        else:
            last_seen_at = self.last_seen_at

        link_version: None | str | Unset
        if isinstance(self.link_version, Unset):
            link_version = UNSET
        else:
            link_version = self.link_version

        sync_cursor: int | None | Unset
        if isinstance(self.sync_cursor, Unset):
            sync_cursor = UNSET
        else:
            sync_cursor = self.sync_cursor

        sync_applied_at: None | str | Unset
        if isinstance(self.sync_applied_at, Unset):
            sync_applied_at = UNSET
        elif isinstance(self.sync_applied_at, datetime.datetime):
            sync_applied_at = self.sync_applied_at.isoformat()
        else:
            sync_applied_at = self.sync_applied_at

        sync_error: None | str | Unset
        if isinstance(self.sync_error, Unset):
            sync_error = UNSET
        else:
            sync_error = self.sync_error

        sync_behind: int | None | Unset
        if isinstance(self.sync_behind, Unset):
            sync_behind = UNSET
        else:
            sync_behind = self.sync_behind

        trunk_id: None | str | Unset
        if isinstance(self.trunk_id, Unset):
            trunk_id = UNSET
        else:
            trunk_id = self.trunk_id

        registrations: dict[str, Any] | None | Unset
        if isinstance(self.registrations, Unset):
            registrations = UNSET
        elif isinstance(self.registrations, SiteRegistrationsView):
            registrations = self.registrations.to_dict()
        else:
            registrations = self.registrations


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "name": name,
            "status": status,
            "created_at": created_at,
            "updated_at": updated_at,
            "link_up": link_up,
            "versions": versions,
            "nodes": nodes,
        })
        if enrolled_at is not UNSET:
            field_dict["enrolled_at"] = enrolled_at
        if certificate_expires_at is not UNSET:
            field_dict["certificate_expires_at"] = certificate_expires_at
        if last_seen_at is not UNSET:
            field_dict["last_seen_at"] = last_seen_at
        if link_version is not UNSET:
            field_dict["link_version"] = link_version
        if sync_cursor is not UNSET:
            field_dict["sync_cursor"] = sync_cursor
        if sync_applied_at is not UNSET:
            field_dict["sync_applied_at"] = sync_applied_at
        if sync_error is not UNSET:
            field_dict["sync_error"] = sync_error
        if sync_behind is not UNSET:
            field_dict["sync_behind"] = sync_behind
        if trunk_id is not UNSET:
            field_dict["trunk_id"] = trunk_id
        if registrations is not UNSET:
            field_dict["registrations"] = registrations

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.site_node import SiteNode # noqa: PLC0415
        from ..models.site_registrations_view import SiteRegistrationsView # noqa: PLC0415
        from ..models.site_response_versions import SiteResponseVersions # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        name = d.pop("name")

        status = SiteStatus(d.pop("status"))




        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        link_up = d.pop("link_up")

        versions = SiteResponseVersions.from_dict(d.pop("versions"))




        nodes = []
        _nodes = d.pop("nodes")
        for nodes_item_data in (_nodes):
            nodes_item = SiteNode.from_dict(nodes_item_data)



            nodes.append(nodes_item)


        def _parse_enrolled_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                enrolled_at_type_0 = datetime.datetime.fromisoformat(data)



                return enrolled_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        enrolled_at = _parse_enrolled_at(d.pop("enrolled_at", UNSET))


        def _parse_certificate_expires_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                certificate_expires_at_type_0 = datetime.datetime.fromisoformat(data)



                return certificate_expires_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        certificate_expires_at = _parse_certificate_expires_at(d.pop("certificate_expires_at", UNSET))


        def _parse_last_seen_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                last_seen_at_type_0 = datetime.datetime.fromisoformat(data)



                return last_seen_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        last_seen_at = _parse_last_seen_at(d.pop("last_seen_at", UNSET))


        def _parse_link_version(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        link_version = _parse_link_version(d.pop("link_version", UNSET))


        def _parse_sync_cursor(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        sync_cursor = _parse_sync_cursor(d.pop("sync_cursor", UNSET))


        def _parse_sync_applied_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                sync_applied_at_type_0 = datetime.datetime.fromisoformat(data)



                return sync_applied_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        sync_applied_at = _parse_sync_applied_at(d.pop("sync_applied_at", UNSET))


        def _parse_sync_error(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        sync_error = _parse_sync_error(d.pop("sync_error", UNSET))


        def _parse_sync_behind(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        sync_behind = _parse_sync_behind(d.pop("sync_behind", UNSET))


        def _parse_trunk_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        trunk_id = _parse_trunk_id(d.pop("trunk_id", UNSET))


        def _parse_registrations(data: object) -> None | SiteRegistrationsView | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                registrations_type_0 = SiteRegistrationsView.from_dict(data)



                return registrations_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | SiteRegistrationsView | Unset, data)

        registrations = _parse_registrations(d.pop("registrations", UNSET))


        site_response = cls(
            id=id,
            tenant_id=tenant_id,
            name=name,
            status=status,
            created_at=created_at,
            updated_at=updated_at,
            link_up=link_up,
            versions=versions,
            nodes=nodes,
            enrolled_at=enrolled_at,
            certificate_expires_at=certificate_expires_at,
            last_seen_at=last_seen_at,
            link_version=link_version,
            sync_cursor=sync_cursor,
            sync_applied_at=sync_applied_at,
            sync_error=sync_error,
            sync_behind=sync_behind,
            trunk_id=trunk_id,
            registrations=registrations,
        )


        site_response.additional_properties = d
        return site_response

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
