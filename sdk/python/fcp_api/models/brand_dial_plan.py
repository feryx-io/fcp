from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.brand_dial_plan_short_codes import BrandDialPlanShortCodes





T = TypeVar("T", bound="BrandDialPlan")



@_attrs_define
class BrandDialPlan:
    """ Hints for the app's dialer: prefixes it takes off what is dialed, and
    short codes it turns into addresses.

        Attributes:
            prefix_strip (list[str] | Unset):
            short_codes (BrandDialPlanShortCodes | Unset):
     """

    prefix_strip: list[str] | Unset = UNSET
    short_codes: BrandDialPlanShortCodes | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.brand_dial_plan_short_codes import BrandDialPlanShortCodes # noqa: PLC0415
        prefix_strip: list[str] | Unset = UNSET
        if not isinstance(self.prefix_strip, Unset):
            prefix_strip = self.prefix_strip



        short_codes: dict[str, Any] | Unset = UNSET
        if not isinstance(self.short_codes, Unset):
            short_codes = self.short_codes.to_dict()


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if prefix_strip is not UNSET:
            field_dict["prefix_strip"] = prefix_strip
        if short_codes is not UNSET:
            field_dict["short_codes"] = short_codes

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.brand_dial_plan_short_codes import BrandDialPlanShortCodes # noqa: PLC0415
        d = dict(src_dict)
        prefix_strip = cast(list[str], d.pop("prefix_strip", UNSET))


        _short_codes = d.pop("short_codes", UNSET)
        short_codes: BrandDialPlanShortCodes | Unset
        if isinstance(_short_codes,  Unset):
            short_codes = UNSET
        else:
            short_codes = BrandDialPlanShortCodes.from_dict(_short_codes)




        brand_dial_plan = cls(
            prefix_strip=prefix_strip,
            short_codes=short_codes,
        )


        brand_dial_plan.additional_properties = d
        return brand_dial_plan

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
