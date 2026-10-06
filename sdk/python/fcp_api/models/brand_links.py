from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="BrandLinks")



@_attrs_define
class BrandLinks:
    """ The tenant's links, each `https://`.

        Attributes:
            support (None | str | Unset):
            privacy (None | str | Unset):
            terms (None | str | Unset):
            website (None | str | Unset):
     """

    support: None | str | Unset = UNSET
    privacy: None | str | Unset = UNSET
    terms: None | str | Unset = UNSET
    website: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        support: None | str | Unset
        if isinstance(self.support, Unset):
            support = UNSET
        else:
            support = self.support

        privacy: None | str | Unset
        if isinstance(self.privacy, Unset):
            privacy = UNSET
        else:
            privacy = self.privacy

        terms: None | str | Unset
        if isinstance(self.terms, Unset):
            terms = UNSET
        else:
            terms = self.terms

        website: None | str | Unset
        if isinstance(self.website, Unset):
            website = UNSET
        else:
            website = self.website


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if support is not UNSET:
            field_dict["support"] = support
        if privacy is not UNSET:
            field_dict["privacy"] = privacy
        if terms is not UNSET:
            field_dict["terms"] = terms
        if website is not UNSET:
            field_dict["website"] = website

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_support(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        support = _parse_support(d.pop("support", UNSET))


        def _parse_privacy(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        privacy = _parse_privacy(d.pop("privacy", UNSET))


        def _parse_terms(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        terms = _parse_terms(d.pop("terms", UNSET))


        def _parse_website(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        website = _parse_website(d.pop("website", UNSET))


        brand_links = cls(
            support=support,
            privacy=privacy,
            terms=terms,
            website=website,
        )


        brand_links.additional_properties = d
        return brand_links

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
