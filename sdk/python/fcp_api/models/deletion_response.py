from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="DeletionResponse")



@_attrs_define
class DeletionResponse:
    """ 
        Attributes:
            id (int):
            class_ (str):
            item (str):
            cause (str):
            actor (str):
            deleted_at (datetime.datetime):
            tenant_id (None | str | Unset):
            detail (Any | Unset):
            request_id (None | str | Unset): The subject request the deletion served.
            api_request_id (None | str | Unset): The API request that made it, as its audit row names it.
            via (None | str | Unset): How the actor reached it: the phone a mailbox's owner called from.
     """

    id: int
    class_: str
    item: str
    cause: str
    actor: str
    deleted_at: datetime.datetime
    tenant_id: None | str | Unset = UNSET
    detail: Any | Unset = UNSET
    request_id: None | str | Unset = UNSET
    api_request_id: None | str | Unset = UNSET
    via: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        class_ = self.class_

        item = self.item

        cause = self.cause

        actor = self.actor

        deleted_at = self.deleted_at.isoformat()

        tenant_id: None | str | Unset
        if isinstance(self.tenant_id, Unset):
            tenant_id = UNSET
        else:
            tenant_id = self.tenant_id

        detail = self.detail

        request_id: None | str | Unset
        if isinstance(self.request_id, Unset):
            request_id = UNSET
        else:
            request_id = self.request_id

        api_request_id: None | str | Unset
        if isinstance(self.api_request_id, Unset):
            api_request_id = UNSET
        else:
            api_request_id = self.api_request_id

        via: None | str | Unset
        if isinstance(self.via, Unset):
            via = UNSET
        else:
            via = self.via


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "class": class_,
            "item": item,
            "cause": cause,
            "actor": actor,
            "deleted_at": deleted_at,
        })
        if tenant_id is not UNSET:
            field_dict["tenant_id"] = tenant_id
        if detail is not UNSET:
            field_dict["detail"] = detail
        if request_id is not UNSET:
            field_dict["request_id"] = request_id
        if api_request_id is not UNSET:
            field_dict["api_request_id"] = api_request_id
        if via is not UNSET:
            field_dict["via"] = via

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        class_ = d.pop("class")

        item = d.pop("item")

        cause = d.pop("cause")

        actor = d.pop("actor")

        deleted_at = datetime.datetime.fromisoformat(d.pop("deleted_at"))




        def _parse_tenant_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        tenant_id = _parse_tenant_id(d.pop("tenant_id", UNSET))


        detail = d.pop("detail", UNSET)

        def _parse_request_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        request_id = _parse_request_id(d.pop("request_id", UNSET))


        def _parse_api_request_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        api_request_id = _parse_api_request_id(d.pop("api_request_id", UNSET))


        def _parse_via(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        via = _parse_via(d.pop("via", UNSET))


        deletion_response = cls(
            id=id,
            class_=class_,
            item=item,
            cause=cause,
            actor=actor,
            deleted_at=deleted_at,
            tenant_id=tenant_id,
            detail=detail,
            request_id=request_id,
            api_request_id=api_request_id,
            via=via,
        )


        deletion_response.additional_properties = d
        return deletion_response

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
