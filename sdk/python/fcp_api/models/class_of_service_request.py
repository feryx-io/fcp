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






T = TypeVar("T", bound="ClassOfServiceRequest")



@_attrs_define
class ClassOfServiceRequest:
    """ What a class is written as. `allowed` unset allows everything;
    `max_calls` unset is two, `null` no limit.

        Attributes:
            name (str):
            allowed (list[DestinationClass] | None | Unset):
            max_calls (int | None | Unset):
            caller_id (CallerIdPolicy | Unset): How a user's caller ID leaves on a trunk.
            is_default (bool | Unset):
     """

    name: str
    allowed: list[DestinationClass] | None | Unset = UNSET
    max_calls: int | None | Unset = UNSET
    caller_id: CallerIdPolicy | Unset = UNSET
    is_default: bool | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        name = self.name

        allowed: list[str] | None | Unset
        if isinstance(self.allowed, Unset):
            allowed = UNSET
        elif isinstance(self.allowed, list):
            allowed = []
            for allowed_type_0_item_data in self.allowed:
                allowed_type_0_item = allowed_type_0_item_data.value
                allowed.append(allowed_type_0_item)


        else:
            allowed = self.allowed

        max_calls: int | None | Unset
        if isinstance(self.max_calls, Unset):
            max_calls = UNSET
        else:
            max_calls = self.max_calls

        caller_id: str | Unset = UNSET
        if not isinstance(self.caller_id, Unset):
            caller_id = self.caller_id.value


        is_default = self.is_default


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "name": name,
        })
        if allowed is not UNSET:
            field_dict["allowed"] = allowed
        if max_calls is not UNSET:
            field_dict["max_calls"] = max_calls
        if caller_id is not UNSET:
            field_dict["caller_id"] = caller_id
        if is_default is not UNSET:
            field_dict["is_default"] = is_default

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        name = d.pop("name")

        def _parse_allowed(data: object) -> list[DestinationClass] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                allowed_type_0 = []
                _allowed_type_0 = data
                for allowed_type_0_item_data in (_allowed_type_0):
                    allowed_type_0_item = DestinationClass(allowed_type_0_item_data)



                    allowed_type_0.append(allowed_type_0_item)

                return allowed_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[DestinationClass] | None | Unset, data)

        allowed = _parse_allowed(d.pop("allowed", UNSET))


        def _parse_max_calls(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        max_calls = _parse_max_calls(d.pop("max_calls", UNSET))


        _caller_id = d.pop("caller_id", UNSET)
        caller_id: CallerIdPolicy | Unset
        if isinstance(_caller_id,  Unset):
            caller_id = UNSET
        else:
            caller_id = CallerIdPolicy(_caller_id)




        is_default = d.pop("is_default", UNSET)

        class_of_service_request = cls(
            name=name,
            allowed=allowed,
            max_calls=max_calls,
            caller_id=caller_id,
            is_default=is_default,
        )


        class_of_service_request.additional_properties = d
        return class_of_service_request

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
