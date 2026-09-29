from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="PageRateResponseDataItem")



@_attrs_define
class PageRateResponseDataItem:
    """ Rate response.

        Attributes:
            id (str):
            rate_plan_id (str):
            prefix (str):
            rate_per_minute (float):
            connect_fee (float | None | Unset):
            description (None | str | Unset):
     """

    id: str
    rate_plan_id: str
    prefix: str
    rate_per_minute: float
    connect_fee: float | None | Unset = UNSET
    description: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        rate_plan_id = self.rate_plan_id

        prefix = self.prefix

        rate_per_minute = self.rate_per_minute

        connect_fee: float | None | Unset
        if isinstance(self.connect_fee, Unset):
            connect_fee = UNSET
        else:
            connect_fee = self.connect_fee

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "rate_plan_id": rate_plan_id,
            "prefix": prefix,
            "rate_per_minute": rate_per_minute,
        })
        if connect_fee is not UNSET:
            field_dict["connect_fee"] = connect_fee
        if description is not UNSET:
            field_dict["description"] = description

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        rate_plan_id = d.pop("rate_plan_id")

        prefix = d.pop("prefix")

        rate_per_minute = d.pop("rate_per_minute")

        def _parse_connect_fee(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        connect_fee = _parse_connect_fee(d.pop("connect_fee", UNSET))


        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        page_rate_response_data_item = cls(
            id=id,
            rate_plan_id=rate_plan_id,
            prefix=prefix,
            rate_per_minute=rate_per_minute,
            connect_fee=connect_fee,
            description=description,
        )


        page_rate_response_data_item.additional_properties = d
        return page_rate_response_data_item

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
