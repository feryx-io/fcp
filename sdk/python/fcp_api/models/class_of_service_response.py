from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.caller_id_policy import CallerIdPolicy
from ..models.destination_class import DestinationClass
from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="ClassOfServiceResponse")



@_attrs_define
class ClassOfServiceResponse:
    """ 
        Attributes:
            id (str):
            tenant_id (str):
            name (str):
            allowed (list[DestinationClass]):
            caller_id (CallerIdPolicy): How a user's caller ID leaves on a trunk.
            video (bool): Its users' calls may carry video.
            is_default (bool):
            updated_at (datetime.datetime):
            max_calls (int | None | Unset):
     """

    id: str
    tenant_id: str
    name: str
    allowed: list[DestinationClass]
    caller_id: CallerIdPolicy
    video: bool
    is_default: bool
    updated_at: datetime.datetime
    max_calls: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        tenant_id = self.tenant_id

        name = self.name

        allowed = []
        for allowed_item_data in self.allowed:
            allowed_item = allowed_item_data.value
            allowed.append(allowed_item)



        caller_id = self.caller_id.value

        video = self.video

        is_default = self.is_default

        updated_at = self.updated_at.isoformat()

        max_calls: int | None | Unset
        if isinstance(self.max_calls, Unset):
            max_calls = UNSET
        else:
            max_calls = self.max_calls


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "name": name,
            "allowed": allowed,
            "caller_id": caller_id,
            "video": video,
            "is_default": is_default,
            "updated_at": updated_at,
        })
        if max_calls is not UNSET:
            field_dict["max_calls"] = max_calls

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        name = d.pop("name")

        allowed = []
        _allowed = d.pop("allowed")
        for allowed_item_data in (_allowed):
            allowed_item = DestinationClass(allowed_item_data)



            allowed.append(allowed_item)


        caller_id = CallerIdPolicy(d.pop("caller_id"))




        video = d.pop("video")

        is_default = d.pop("is_default")

        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_max_calls(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        max_calls = _parse_max_calls(d.pop("max_calls", UNSET))


        class_of_service_response = cls(
            id=id,
            tenant_id=tenant_id,
            name=name,
            allowed=allowed,
            caller_id=caller_id,
            video=video,
            is_default=is_default,
            updated_at=updated_at,
            max_calls=max_calls,
        )


        class_of_service_response.additional_properties = d
        return class_of_service_response

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
