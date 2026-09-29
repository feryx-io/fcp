from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.page_rate_limit_info_data_item_counts import PageRateLimitInfoDataItemCounts





T = TypeVar("T", bound="PageRateLimitInfoDataItem")



@_attrs_define
class PageRateLimitInfoDataItem:
    """ Rate limit state response.

        Attributes:
            dialog_id (str):
            window_start (str):
            counts (PageRateLimitInfoDataItemCounts):
     """

    dialog_id: str
    window_start: str
    counts: PageRateLimitInfoDataItemCounts
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.page_rate_limit_info_data_item_counts import PageRateLimitInfoDataItemCounts # noqa: PLC0415
        dialog_id = self.dialog_id

        window_start = self.window_start

        counts = self.counts.to_dict()


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "dialog_id": dialog_id,
            "window_start": window_start,
            "counts": counts,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.page_rate_limit_info_data_item_counts import PageRateLimitInfoDataItemCounts # noqa: PLC0415
        d = dict(src_dict)
        dialog_id = d.pop("dialog_id")

        window_start = d.pop("window_start")

        counts = PageRateLimitInfoDataItemCounts.from_dict(d.pop("counts"))




        page_rate_limit_info_data_item = cls(
            dialog_id=dialog_id,
            window_start=window_start,
            counts=counts,
        )


        page_rate_limit_info_data_item.additional_properties = d
        return page_rate_limit_info_data_item

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
