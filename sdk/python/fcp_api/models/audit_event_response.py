from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="AuditEventResponse")



@_attrs_define
class AuditEventResponse:
    """ 
        Attributes:
            id (int):
            actor (str): Who acted. A tenant's readers see an actor from outside it (a
                platform administrator) as `platform administrator`.
            actor_kind (str):
            method (str):
            path (str):
            status (int):
            action (str):
            occurred_at (datetime.datetime):
            tenant_id (None | str | Unset): The tenant acted on; `null` for the platform's own actions.
            actor_tenant_id (None | str | Unset): The actor's own tenant, `null` for the static token. Shown only to a
                platform administrator.
            detail (Any | Unset):
            source_ip (None | str | Unset):
     """

    id: int
    actor: str
    actor_kind: str
    method: str
    path: str
    status: int
    action: str
    occurred_at: datetime.datetime
    tenant_id: None | str | Unset = UNSET
    actor_tenant_id: None | str | Unset = UNSET
    detail: Any | Unset = UNSET
    source_ip: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        actor = self.actor

        actor_kind = self.actor_kind

        method = self.method

        path = self.path

        status = self.status

        action = self.action

        occurred_at = self.occurred_at.isoformat()

        tenant_id: None | str | Unset
        if isinstance(self.tenant_id, Unset):
            tenant_id = UNSET
        else:
            tenant_id = self.tenant_id

        actor_tenant_id: None | str | Unset
        if isinstance(self.actor_tenant_id, Unset):
            actor_tenant_id = UNSET
        else:
            actor_tenant_id = self.actor_tenant_id

        detail = self.detail

        source_ip: None | str | Unset
        if isinstance(self.source_ip, Unset):
            source_ip = UNSET
        else:
            source_ip = self.source_ip


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "actor": actor,
            "actor_kind": actor_kind,
            "method": method,
            "path": path,
            "status": status,
            "action": action,
            "occurred_at": occurred_at,
        })
        if tenant_id is not UNSET:
            field_dict["tenant_id"] = tenant_id
        if actor_tenant_id is not UNSET:
            field_dict["actor_tenant_id"] = actor_tenant_id
        if detail is not UNSET:
            field_dict["detail"] = detail
        if source_ip is not UNSET:
            field_dict["source_ip"] = source_ip

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        actor = d.pop("actor")

        actor_kind = d.pop("actor_kind")

        method = d.pop("method")

        path = d.pop("path")

        status = d.pop("status")

        action = d.pop("action")

        occurred_at = datetime.datetime.fromisoformat(d.pop("occurred_at"))




        def _parse_tenant_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        tenant_id = _parse_tenant_id(d.pop("tenant_id", UNSET))


        def _parse_actor_tenant_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        actor_tenant_id = _parse_actor_tenant_id(d.pop("actor_tenant_id", UNSET))


        detail = d.pop("detail", UNSET)

        def _parse_source_ip(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        source_ip = _parse_source_ip(d.pop("source_ip", UNSET))


        audit_event_response = cls(
            id=id,
            actor=actor,
            actor_kind=actor_kind,
            method=method,
            path=path,
            status=status,
            action=action,
            occurred_at=occurred_at,
            tenant_id=tenant_id,
            actor_tenant_id=actor_tenant_id,
            detail=detail,
            source_ip=source_ip,
        )


        audit_event_response.additional_properties = d
        return audit_event_response

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
