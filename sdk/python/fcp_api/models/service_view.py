from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.service_check_view import ServiceCheckView
  from ..models.service_view_load import ServiceViewLoad





T = TypeVar("T", bound="ServiceView")



@_attrs_define
class ServiceView:
    """ One service.

        Attributes:
            id (str):
            kind (str):
            host (str):
            version (str):
            state (str): `serving`, `degraded`, `draining`, `stopping`, `down` (its beacon
                went), `stopped` (it said it was stopping), or `unknown` (Redis, or
                the site's link, cannot be read now).
            first_seen (datetime.datetime):
            last_seen (datetime.datetime):
            member (bool): Also a cluster member.
            checks (list[ServiceCheckView]):
            load (ServiceViewLoad):
            site_id (None | str | Unset): The site it runs at; null for the platform's own.
            site_name (None | str | Unset):
            down_since (datetime.datetime | None | Unset):
            started_at (datetime.datetime | None | Unset): When the process started, from its last beacon.
            region (None | str | Unset):
            http_address (None | str | Unset):
            sip_address (None | str | Unset):
            public_url (None | str | Unset):
     """

    id: str
    kind: str
    host: str
    version: str
    state: str
    first_seen: datetime.datetime
    last_seen: datetime.datetime
    member: bool
    checks: list[ServiceCheckView]
    load: ServiceViewLoad
    site_id: None | str | Unset = UNSET
    site_name: None | str | Unset = UNSET
    down_since: datetime.datetime | None | Unset = UNSET
    started_at: datetime.datetime | None | Unset = UNSET
    region: None | str | Unset = UNSET
    http_address: None | str | Unset = UNSET
    sip_address: None | str | Unset = UNSET
    public_url: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.service_check_view import ServiceCheckView # noqa: PLC0415
        from ..models.service_view_load import ServiceViewLoad # noqa: PLC0415
        id = self.id

        kind = self.kind

        host = self.host

        version = self.version

        state = self.state

        first_seen = self.first_seen.isoformat()

        last_seen = self.last_seen.isoformat()

        member = self.member

        checks = []
        for checks_item_data in self.checks:
            checks_item = checks_item_data.to_dict()
            checks.append(checks_item)



        load = self.load.to_dict()

        site_id: None | str | Unset
        if isinstance(self.site_id, Unset):
            site_id = UNSET
        else:
            site_id = self.site_id

        site_name: None | str | Unset
        if isinstance(self.site_name, Unset):
            site_name = UNSET
        else:
            site_name = self.site_name

        down_since: None | str | Unset
        if isinstance(self.down_since, Unset):
            down_since = UNSET
        elif isinstance(self.down_since, datetime.datetime):
            down_since = self.down_since.isoformat()
        else:
            down_since = self.down_since

        started_at: None | str | Unset
        if isinstance(self.started_at, Unset):
            started_at = UNSET
        elif isinstance(self.started_at, datetime.datetime):
            started_at = self.started_at.isoformat()
        else:
            started_at = self.started_at

        region: None | str | Unset
        if isinstance(self.region, Unset):
            region = UNSET
        else:
            region = self.region

        http_address: None | str | Unset
        if isinstance(self.http_address, Unset):
            http_address = UNSET
        else:
            http_address = self.http_address

        sip_address: None | str | Unset
        if isinstance(self.sip_address, Unset):
            sip_address = UNSET
        else:
            sip_address = self.sip_address

        public_url: None | str | Unset
        if isinstance(self.public_url, Unset):
            public_url = UNSET
        else:
            public_url = self.public_url


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "kind": kind,
            "host": host,
            "version": version,
            "state": state,
            "first_seen": first_seen,
            "last_seen": last_seen,
            "member": member,
            "checks": checks,
            "load": load,
        })
        if site_id is not UNSET:
            field_dict["site_id"] = site_id
        if site_name is not UNSET:
            field_dict["site_name"] = site_name
        if down_since is not UNSET:
            field_dict["down_since"] = down_since
        if started_at is not UNSET:
            field_dict["started_at"] = started_at
        if region is not UNSET:
            field_dict["region"] = region
        if http_address is not UNSET:
            field_dict["http_address"] = http_address
        if sip_address is not UNSET:
            field_dict["sip_address"] = sip_address
        if public_url is not UNSET:
            field_dict["public_url"] = public_url

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.service_check_view import ServiceCheckView # noqa: PLC0415
        from ..models.service_view_load import ServiceViewLoad # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        kind = d.pop("kind")

        host = d.pop("host")

        version = d.pop("version")

        state = d.pop("state")

        first_seen = datetime.datetime.fromisoformat(d.pop("first_seen"))




        last_seen = datetime.datetime.fromisoformat(d.pop("last_seen"))




        member = d.pop("member")

        checks = []
        _checks = d.pop("checks")
        for checks_item_data in (_checks):
            checks_item = ServiceCheckView.from_dict(checks_item_data)



            checks.append(checks_item)


        load = ServiceViewLoad.from_dict(d.pop("load"))




        def _parse_site_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        site_id = _parse_site_id(d.pop("site_id", UNSET))


        def _parse_site_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        site_name = _parse_site_name(d.pop("site_name", UNSET))


        def _parse_down_since(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                down_since_type_0 = datetime.datetime.fromisoformat(data)



                return down_since_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        down_since = _parse_down_since(d.pop("down_since", UNSET))


        def _parse_started_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                started_at_type_0 = datetime.datetime.fromisoformat(data)



                return started_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        started_at = _parse_started_at(d.pop("started_at", UNSET))


        def _parse_region(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        region = _parse_region(d.pop("region", UNSET))


        def _parse_http_address(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        http_address = _parse_http_address(d.pop("http_address", UNSET))


        def _parse_sip_address(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        sip_address = _parse_sip_address(d.pop("sip_address", UNSET))


        def _parse_public_url(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        public_url = _parse_public_url(d.pop("public_url", UNSET))


        service_view = cls(
            id=id,
            kind=kind,
            host=host,
            version=version,
            state=state,
            first_seen=first_seen,
            last_seen=last_seen,
            member=member,
            checks=checks,
            load=load,
            site_id=site_id,
            site_name=site_name,
            down_since=down_since,
            started_at=started_at,
            region=region,
            http_address=http_address,
            sip_address=sip_address,
            public_url=public_url,
        )


        service_view.additional_properties = d
        return service_view

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
