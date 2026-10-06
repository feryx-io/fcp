from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.page_brand_asset_response_data_item import PageBrandAssetResponseDataItem





T = TypeVar("T", bound="PageBrandAssetResponse")



@_attrs_define
class PageBrandAssetResponse:
    """ A page of a list.

        Attributes:
            data (list[PageBrandAssetResponseDataItem]):
            next_cursor (None | str | Unset): The cursor for the next page; null on the last.
            total (int | None | Unset): How many items the whole list holds, where that is known.
     """

    data: list[PageBrandAssetResponseDataItem]
    next_cursor: None | str | Unset = UNSET
    total: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.page_brand_asset_response_data_item import PageBrandAssetResponseDataItem # noqa: PLC0415
        data = []
        for data_item_data in self.data:
            data_item = data_item_data.to_dict()
            data.append(data_item)



        next_cursor: None | str | Unset
        if isinstance(self.next_cursor, Unset):
            next_cursor = UNSET
        else:
            next_cursor = self.next_cursor

        total: int | None | Unset
        if isinstance(self.total, Unset):
            total = UNSET
        else:
            total = self.total


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "data": data,
        })
        if next_cursor is not UNSET:
            field_dict["next_cursor"] = next_cursor
        if total is not UNSET:
            field_dict["total"] = total

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.page_brand_asset_response_data_item import PageBrandAssetResponseDataItem # noqa: PLC0415
        d = dict(src_dict)
        data = []
        _data = d.pop("data")
        for data_item_data in (_data):
            data_item = PageBrandAssetResponseDataItem.from_dict(data_item_data)



            data.append(data_item)


        def _parse_next_cursor(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        next_cursor = _parse_next_cursor(d.pop("next_cursor", UNSET))


        def _parse_total(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        total = _parse_total(d.pop("total", UNSET))


        page_brand_asset_response = cls(
            data=data,
            next_cursor=next_cursor,
            total=total,
        )


        page_brand_asset_response.additional_properties = d
        return page_brand_asset_response

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
